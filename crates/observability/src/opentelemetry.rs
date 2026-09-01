use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{Duration, UNIX_EPOCH};

use opentelemetry::logs::{LogRecord, Logger, Severity as OtelSeverity};
use opentelemetry::metrics::{Counter, Gauge, Histogram, Meter};

use crate::{
    EventName, MetricEnvelopeV1, MetricKind, ObservabilityError, Severity, TelemetryEnvelopeV1,
    TelemetryGuarantees, TelemetrySink, validate_envelope, validate_metric, validate_tenant_id,
};

const EVENT_NAME: &str = "rust_factory.telemetry";
const EVENT_TARGET: &str = "rust_factory.observability";

pub struct OpenTelemetryLogsSink<L> {
    logger: L,
}
impl<L> OpenTelemetryLogsSink<L> {
    #[must_use]
    pub const fn new(logger: L) -> Self {
        Self { logger }
    }
}
impl<L> TelemetrySink for OpenTelemetryLogsSink<L>
where
    L: Logger + Send + Sync,
{
    fn emit(&self, envelope: TelemetryEnvelopeV1) -> Result<(), ObservabilityError> {
        validate_envelope(&envelope)?;
        let timestamp = UNIX_EPOCH
            .checked_add(Duration::from_nanos(envelope.timestamp.as_unix_nanos()))
            .ok_or(ObservabilityError::InvalidEvent)?;
        let severity = severity(envelope.event.severity);
        if !self
            .logger
            .event_enabled(severity, EVENT_TARGET, Some(EVENT_NAME))
        {
            return Ok(());
        }
        let mut record = self.logger.create_log_record();
        record.set_event_name(EVENT_NAME);
        record.set_target(EVENT_TARGET);
        record.set_timestamp(timestamp);
        record.set_severity_number(severity);
        record.set_severity_text(severity.name());
        record.add_attribute(
            "rust_factory.event_name",
            envelope.event.name.as_str().to_owned(),
        );
        record.add_attribute(
            "rust_factory.event_target",
            envelope.event.target.as_str().to_owned(),
        );
        self.logger.emit(record);
        Ok(())
    }

    fn guarantees(&self) -> TelemetryGuarantees {
        TelemetryGuarantees {
            durable_across_restart: false,
            visible_across_processes: false,
            delivery_confirmed: false,
            queryable: false,
            // This adapter wraps an injected `Logger`/`Meter` whose
            // underlying I/O behavior is composition-root-owned and
            // unobservable here (this brick SHALL NOT construct an SDK,
            // exporter, or runtime). "Unknown" is not a safe substitute
            // for "non-blocking", so this reports the conservative value.
            may_block: true,
        }
    }
}

const fn severity(value: Severity) -> OtelSeverity {
    match value {
        Severity::Trace => OtelSeverity::Trace,
        Severity::Debug => OtelSeverity::Debug,
        Severity::Info => OtelSeverity::Info,
        Severity::Warn => OtelSeverity::Warn,
        Severity::Error => OtelSeverity::Error,
    }
}

/// One cached OTEL metric instrument, keyed by `(EventName, MetricKind)`. OTEL
/// instrument metadata (in particular `unit`) is fixed at creation, so the
/// cache's first-seen `unit` for a given name/kind pair is retained; a later
/// `emit` call with a different `unit` for the same cached pair does not
/// update the instrument.
enum Instrument {
    Counter(Counter<f64>),
    Gauge(Gauge<f64>),
    Histogram(Histogram<f64>),
}

/// A minimal, framework-neutral OTEL metric export adapter.
///
/// Receives an injected `opentelemetry::metrics::Meter` only. It SHALL NOT
/// construct an SDK, exporter, runtime, batch processor, network client, or
/// shutdown hook; those belong to a composition binary, matching
/// [`OpenTelemetryLogsSink`]'s existing rule.
///
/// Egress contains only the validated metric name (as the OTEL instrument
/// name), kind (mapped to the corresponding instrument type), value, and unit
/// if present. `tenant_id` and caller-supplied `attributes` are never
/// exported: there is no type-level trust distinction between
/// `MetricEventV1.attributes` and log `attributes` (both are
/// `BTreeMap<String, String>` under the identical validation rule), so this
/// matches the log adapter's metadata-only data-minimization boundary rather
/// than relaxing it.
pub struct OpenTelemetryMetricSink {
    meter: Meter,
    instruments: Mutex<BTreeMap<EventName, (MetricKind, Instrument)>>,
}
impl OpenTelemetryMetricSink {
    #[must_use]
    pub fn new(meter: Meter) -> Self {
        Self {
            meter,
            instruments: Mutex::new(BTreeMap::new()),
        }
    }

    /// Returns the cached instrument for `name`, creating and inserting one
    /// for `(name, kind)` on first use. Returns `InvalidMetric` if `name` is
    /// already cached under a different `kind`, rather than silently
    /// reusing a mismatched instrument. `unit` is applied only on first
    /// creation; OTEL instrument metadata is fixed once built, so a later
    /// call's differing `unit` for an already-cached name is not applied.
    fn instrument_for(
        &self,
        name: &EventName,
        kind: MetricKind,
        unit: Option<&str>,
    ) -> Result<InstrumentHandle, ObservabilityError> {
        let mut instruments = self
            .instruments
            .lock()
            .map_err(|_| ObservabilityError::AdapterFailure)?;
        if let Some((existing_kind, existing)) = instruments.get(name) {
            return if *existing_kind == kind {
                Ok(existing.handle())
            } else {
                Err(ObservabilityError::InvalidMetric)
            };
        }
        let instrument_name = name.as_str().to_owned();
        let instrument = match kind {
            MetricKind::Counter => {
                let mut builder = self.meter.f64_counter(instrument_name);
                if let Some(unit) = unit {
                    builder = builder.with_unit(unit.to_owned());
                }
                Instrument::Counter(builder.build())
            }
            MetricKind::Gauge => {
                let mut builder = self.meter.f64_gauge(instrument_name);
                if let Some(unit) = unit {
                    builder = builder.with_unit(unit.to_owned());
                }
                Instrument::Gauge(builder.build())
            }
            MetricKind::Histogram => {
                let mut builder = self.meter.f64_histogram(instrument_name);
                if let Some(unit) = unit {
                    builder = builder.with_unit(unit.to_owned());
                }
                Instrument::Histogram(builder.build())
            }
        };
        let handle = instrument.handle();
        instruments.insert(name.clone(), (kind, instrument));
        Ok(handle)
    }
}
impl Instrument {
    fn handle(&self) -> InstrumentHandle {
        match self {
            Self::Counter(counter) => InstrumentHandle::Counter(counter.clone()),
            Self::Gauge(gauge) => InstrumentHandle::Gauge(gauge.clone()),
            Self::Histogram(histogram) => InstrumentHandle::Histogram(histogram.clone()),
        }
    }
}
impl crate::MetricSink for OpenTelemetryMetricSink {
    fn emit(&self, envelope: MetricEnvelopeV1) -> Result<(), ObservabilityError> {
        validate_tenant_id(envelope.tenant_id.as_str())?;
        validate_metric(&envelope.metric)?;
        let handle = self.instrument_for(
            &envelope.metric.name,
            envelope.metric.kind,
            envelope.metric.unit.as_deref(),
        )?;
        match handle {
            InstrumentHandle::Counter(counter) => counter.add(envelope.metric.value, &[]),
            InstrumentHandle::Gauge(gauge) => gauge.record(envelope.metric.value, &[]),
            InstrumentHandle::Histogram(histogram) => {
                histogram.record(envelope.metric.value, &[]);
            }
        }
        Ok(())
    }

    fn guarantees(&self) -> TelemetryGuarantees {
        TelemetryGuarantees {
            durable_across_restart: false,
            visible_across_processes: false,
            delivery_confirmed: false,
            queryable: false,
            // This adapter wraps an injected `Logger`/`Meter` whose
            // underlying I/O behavior is composition-root-owned and
            // unobservable here (this brick SHALL NOT construct an SDK,
            // exporter, or runtime). "Unknown" is not a safe substitute
            // for "non-blocking", so this reports the conservative value.
            may_block: true,
        }
    }
}

/// A cloned handle to one cached instrument, returned out of the mutex guard
/// so `emit` can call `.add`/`.record` without holding the lock.
enum InstrumentHandle {
    Counter(Counter<f64>),
    Gauge(Gauge<f64>),
    Histogram(Histogram<f64>),
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use opentelemetry::KeyValue;
    use opentelemetry::metrics::{
        Counter as OtelCounter, Gauge as OtelGauge, Histogram as OtelHistogram, InstrumentBuilder,
        InstrumentProvider, Meter, SyncInstrument,
    };

    use super::*;
    use crate::{MetricEnvelopeV1, MetricEventV1, MetricSink};
    use std::collections::BTreeMap;

    #[derive(Clone, Default)]
    struct NoopInstrument;
    impl SyncInstrument<f64> for NoopInstrument {
        fn measure(&self, _measurement: f64, _attributes: &[KeyValue]) {}
    }

    #[derive(Clone, Default)]
    struct NoopInstrumentProvider;
    impl InstrumentProvider for NoopInstrumentProvider {
        fn f64_counter(
            &self,
            _builder: InstrumentBuilder<'_, OtelCounter<f64>>,
        ) -> OtelCounter<f64> {
            OtelCounter::new(Arc::new(NoopInstrument))
        }
        fn f64_gauge(&self, _builder: InstrumentBuilder<'_, OtelGauge<f64>>) -> OtelGauge<f64> {
            OtelGauge::new(Arc::new(NoopInstrument))
        }
        fn f64_histogram(
            &self,
            _builder: opentelemetry::metrics::HistogramBuilder<'_, OtelHistogram<f64>>,
        ) -> OtelHistogram<f64> {
            OtelHistogram::new(Arc::new(NoopInstrument))
        }
    }

    fn noop_meter() -> Meter {
        Meter::new(Arc::new(NoopInstrumentProvider))
    }

    fn metric_envelope(name: &str, value: f64) -> MetricEnvelopeV1 {
        MetricEnvelopeV1 {
            tenant_id: crate::TenantId::new("tenant").expect("tenant"),
            metric: MetricEventV1::new(
                EventName::new(name).expect("name"),
                MetricKind::Counter,
                value,
                None,
                BTreeMap::new(),
                crate::Timestamp::from_unix_nanos(1),
            )
            .expect("metric"),
        }
    }

    /// Mirrors `LocalTelemetry`'s
    /// `poisoned_state_fails_closed_for_reads_and_writes` coverage
    /// (`src/local.rs`): a panic while holding the instrument-cache lock
    /// must poison it, and the sink must fail closed with
    /// `AdapterFailure` on the next `emit`, not panic or silently recover.
    #[test]
    fn poisoned_instrument_cache_fails_closed_on_the_next_emit() {
        let sink = Arc::new(OpenTelemetryMetricSink::new(noop_meter()));
        sink.emit(metric_envelope("requests_total", 1.0))
            .expect("first emit establishes the cache");

        let poisoner = Arc::clone(&sink);
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.instruments.lock().expect("lock");
            panic!("poison instrument cache for deterministic failure test");
        })
        .join();

        assert_eq!(
            sink.emit(metric_envelope("requests_total", 2.0)),
            Err(ObservabilityError::AdapterFailure)
        );
    }
}
