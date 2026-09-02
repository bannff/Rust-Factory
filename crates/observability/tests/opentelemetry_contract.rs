#![cfg(feature = "opentelemetry")]

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use observability::opentelemetry::OpenTelemetryLogsSink;
use observability::{
    EventName, EventTarget, Severity, TelemetryEnvelopeV1, TelemetryEventV1, TelemetryGuarantees,
    TelemetrySink, TenantId, Timestamp,
};
use opentelemetry::logs::{AnyValue, LogRecord, Logger, Severity as OtelSeverity};
use opentelemetry::{Key, SpanId, TraceFlags, TraceId};

#[derive(Clone, Debug, Default, PartialEq)]
struct Recorded {
    event_name: Option<&'static str>,
    target: Option<String>,
    timestamp: Option<SystemTime>,
    observed_timestamp: Option<SystemTime>,
    severity_text: Option<&'static str>,
    severity: Option<OtelSeverity>,
    body: Option<AnyValue>,
    attributes: Vec<(Key, AnyValue)>,
}
impl LogRecord for Recorded {
    fn set_event_name(&mut self, value: &'static str) {
        self.event_name = Some(value);
    }
    fn set_target<T: Into<Cow<'static, str>>>(&mut self, value: T) {
        self.target = Some(value.into().into_owned());
    }
    fn set_timestamp(&mut self, value: SystemTime) {
        self.timestamp = Some(value);
    }
    fn set_observed_timestamp(&mut self, value: SystemTime) {
        self.observed_timestamp = Some(value);
    }
    fn set_severity_text(&mut self, value: &'static str) {
        self.severity_text = Some(value);
    }
    fn set_severity_number(&mut self, value: OtelSeverity) {
        self.severity = Some(value);
    }
    fn set_body(&mut self, value: AnyValue) {
        self.body = Some(value);
    }
    fn add_attributes<I, K, V>(&mut self, values: I)
    where
        I: IntoIterator<Item = (K, V)>,
        K: Into<Key>,
        V: Into<AnyValue>,
    {
        self.attributes
            .extend(values.into_iter().map(|(k, v)| (k.into(), v.into())));
    }
    fn add_attribute<K: Into<Key>, V: Into<AnyValue>>(&mut self, key: K, value: V) {
        self.attributes.push((key.into(), value.into()));
    }
    fn set_trace_context(&mut self, _: TraceId, _: SpanId, _: Option<TraceFlags>) {}
}

type EnabledCall = (OtelSeverity, String, Option<String>);

#[derive(Clone)]
struct RecordingLogger {
    enabled: bool,
    enabled_calls: Arc<Mutex<Vec<EnabledCall>>>,
    emitted: Arc<Mutex<Vec<Recorded>>>,
}
impl Logger for RecordingLogger {
    type LogRecord = Recorded;
    fn create_log_record(&self) -> Recorded {
        Recorded::default()
    }
    fn emit(&self, record: Recorded) {
        self.emitted.lock().expect("emitted").push(record);
    }
    fn event_enabled(&self, level: OtelSeverity, target: &str, name: Option<&str>) -> bool {
        self.enabled_calls.lock().expect("calls").push((
            level,
            target.to_owned(),
            name.map(str::to_owned),
        ));
        self.enabled
    }
}
fn logger(enabled: bool) -> RecordingLogger {
    RecordingLogger {
        enabled,
        enabled_calls: Arc::new(Mutex::new(vec![])),
        emitted: Arc::new(Mutex::new(vec![])),
    }
}
fn envelope(severity: Severity) -> TelemetryEnvelopeV1 {
    TelemetryEnvelopeV1 {
        tenant_id: TenantId::new("tenant").expect("tenant"),
        timestamp: Timestamp::from_unix_nanos(42),
        event: TelemetryEventV1::new(
            EventName::new("job.finished").expect("name"),
            EventTarget::new("worker").expect("target"),
            severity,
            "done",
            BTreeMap::from([
                ("custom".to_owned(), "value".to_owned()),
                ("rust_factory.tenant_id".to_owned(), "forged".to_owned()),
                ("rust_factory.other".to_owned(), "forged".to_owned()),
            ]),
        )
        .expect("event"),
    }
}
fn string(value: &AnyValue) -> &str {
    match value {
        AnyValue::String(value) => value.as_str(),
        other => panic!("expected string, got {other:?}"),
    }
}

#[test]
fn adapter_projects_only_bounded_operational_metadata() {
    for (core, otel, text) in [
        (Severity::Trace, OtelSeverity::Trace, "TRACE"),
        (Severity::Debug, OtelSeverity::Debug, "DEBUG"),
        (Severity::Info, OtelSeverity::Info, "INFO"),
        (Severity::Warn, OtelSeverity::Warn, "WARN"),
        (Severity::Error, OtelSeverity::Error, "ERROR"),
    ] {
        let logger = logger(true);
        OpenTelemetryLogsSink::new(logger.clone())
            .emit(envelope(core))
            .expect("submit");
        assert_eq!(
            logger.enabled_calls.lock().expect("calls").as_slice(),
            &[(
                otel,
                "rust_factory.observability".to_owned(),
                Some("rust_factory.telemetry".to_owned())
            )]
        );
        let records = logger.emitted.lock().expect("emitted");
        let record = &records[0];
        assert_eq!(record.event_name, Some("rust_factory.telemetry"));
        assert_eq!(record.target.as_deref(), Some("rust_factory.observability"));
        let expected = UNIX_EPOCH
            .checked_add(Duration::from_nanos(42))
            .expect("timestamp");
        assert_eq!(record.timestamp, Some(expected));
        assert_eq!(record.observed_timestamp, None);
        assert_eq!(record.severity, Some(otel));
        assert_eq!(record.severity_text, Some(text));
        assert!(record.body.is_none());
        let attributes = record
            .attributes
            .iter()
            .map(|(key, value)| (key.as_str(), string(value)))
            .collect::<BTreeMap<_, _>>();
        assert!(!attributes.contains_key("rust_factory.tenant_id"));
        assert_eq!(
            attributes.get("rust_factory.event_name"),
            Some(&"job.finished")
        );
        assert_eq!(attributes.get("rust_factory.event_target"), Some(&"worker"));
        assert!(!attributes.contains_key("custom"));
        assert!(!attributes.contains_key("rust_factory.other"));
        let encoded = format!("{record:?}");
        for prohibited in ["tenant", "done", "value", "forged"] {
            assert!(!encoded.contains(prohibited), "export leaked {prohibited}");
        }
    }
}

#[test]
fn disabled_events_are_not_created_or_emitted_and_guarantees_are_submit_only() {
    let logger = logger(false);
    let sink = OpenTelemetryLogsSink::new(logger.clone());
    sink.emit(envelope(Severity::Info))
        .expect("disabled is accepted");
    assert!(logger.emitted.lock().expect("emitted").is_empty());
    assert_eq!(
        sink.guarantees(),
        TelemetryGuarantees {
            durable_across_restart: false,
            visible_across_processes: false,
            delivery_confirmed: false,
            queryable: false,
            may_block: true,
        }
    );
}

mod metric_sink {
    use std::sync::{Arc, Mutex};

    use observability::opentelemetry::OpenTelemetryMetricSink;
    use observability::{
        EventName, MetricEnvelopeV1, MetricEventV1, MetricKind, MetricSink, ObservabilityError,
        TelemetryGuarantees, TenantId, Timestamp,
    };
    use opentelemetry::KeyValue;
    use opentelemetry::metrics::{
        Counter, Gauge, Histogram, InstrumentBuilder, InstrumentProvider, Meter, SyncInstrument,
    };

    #[derive(Clone, Debug, Default)]
    struct RecordedMeasurement {
        instrument_name: String,
        value: f64,
        attributes: Vec<KeyValue>,
    }

    #[derive(Clone, Default)]
    struct RecordingSyncInstrument {
        name: String,
        measurements: Arc<Mutex<Vec<RecordedMeasurement>>>,
    }
    impl SyncInstrument<f64> for RecordingSyncInstrument {
        fn measure(&self, measurement: f64, attributes: &[KeyValue]) {
            self.measurements
                .lock()
                .expect("measurements")
                .push(RecordedMeasurement {
                    instrument_name: self.name.clone(),
                    value: measurement,
                    attributes: attributes.to_vec(),
                });
        }
    }

    #[derive(Clone, Default)]
    struct RecordingInstrumentProvider {
        created: Arc<Mutex<Vec<String>>>,
        measurements: Arc<Mutex<Vec<RecordedMeasurement>>>,
    }
    impl InstrumentProvider for RecordingInstrumentProvider {
        fn f64_counter(&self, builder: InstrumentBuilder<'_, Counter<f64>>) -> Counter<f64> {
            self.created
                .lock()
                .expect("created")
                .push(builder.name.to_string());
            Counter::new(Arc::new(RecordingSyncInstrument {
                name: builder.name.to_string(),
                measurements: self.measurements.clone(),
            }))
        }
        fn f64_gauge(&self, builder: InstrumentBuilder<'_, Gauge<f64>>) -> Gauge<f64> {
            self.created
                .lock()
                .expect("created")
                .push(builder.name.to_string());
            Gauge::new(Arc::new(RecordingSyncInstrument {
                name: builder.name.to_string(),
                measurements: self.measurements.clone(),
            }))
        }
        fn f64_histogram(
            &self,
            builder: opentelemetry::metrics::HistogramBuilder<'_, Histogram<f64>>,
        ) -> Histogram<f64> {
            self.created
                .lock()
                .expect("created")
                .push(builder.name.to_string());
            Histogram::new(Arc::new(RecordingSyncInstrument {
                name: builder.name.to_string(),
                measurements: self.measurements.clone(),
            }))
        }
    }

    fn meter() -> (Meter, RecordingInstrumentProvider) {
        let provider = RecordingInstrumentProvider::default();
        (Meter::new(Arc::new(provider.clone())), provider)
    }

    fn metric_envelope(
        name: &str,
        kind: MetricKind,
        value: f64,
        unit: Option<&str>,
    ) -> MetricEnvelopeV1 {
        MetricEnvelopeV1 {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            metric: MetricEventV1::new(
                EventName::new(name).expect("name"),
                kind,
                value,
                unit.map(str::to_owned),
                std::collections::BTreeMap::from([("forged".to_owned(), "leaked".to_owned())]),
                Timestamp::from_unix_nanos(1),
            )
            .expect("metric"),
        }
    }

    #[test]
    fn each_kind_maps_to_the_correct_instrument_type_and_records_the_value() {
        for (kind, name) in [
            (MetricKind::Counter, "requests_total"),
            (MetricKind::Gauge, "queue_depth"),
            (MetricKind::Histogram, "latency_ms"),
        ] {
            let (meter, provider) = meter();
            let sink = OpenTelemetryMetricSink::new(meter);
            sink.emit(metric_envelope(name, kind, 3.5, Some("ms")))
                .expect("emit");
            let created = provider.created.lock().expect("created");
            assert_eq!(created.as_slice(), &[name.to_owned()]);
            let measurements = provider.measurements.lock().expect("measurements");
            assert_eq!(measurements.len(), 1);
            assert_eq!(measurements[0].instrument_name, name);
            assert!((measurements[0].value - 3.5).abs() < f64::EPSILON);
        }
    }

    #[test]
    fn tenant_and_caller_attributes_are_never_exported() {
        let (meter, provider) = meter();
        let sink = OpenTelemetryMetricSink::new(meter);
        sink.emit(metric_envelope(
            "requests_total",
            MetricKind::Counter,
            1.0,
            None,
        ))
        .expect("emit");
        let measurements = provider.measurements.lock().expect("measurements");
        assert!(
            measurements[0].attributes.is_empty(),
            "attributes must never be forwarded"
        );
        // The metric's own caller-supplied attribute value ("leaked") and the
        // tenant id ("tenant") must not appear anywhere in the exported record.
        let encoded = format!("{:?}", measurements[0]);
        for prohibited in ["tenant", "leaked", "forged"] {
            assert!(!encoded.contains(prohibited), "export leaked {prohibited}");
        }
    }

    #[test]
    fn instrument_is_created_once_and_reused_across_repeated_emits() {
        let (meter, provider) = meter();
        let sink = OpenTelemetryMetricSink::new(meter);
        for _ in 0..3 {
            sink.emit(metric_envelope(
                "requests_total",
                MetricKind::Counter,
                1.0,
                None,
            ))
            .expect("emit");
        }
        assert_eq!(provider.created.lock().expect("created").len(), 1);
        assert_eq!(provider.measurements.lock().expect("measurements").len(), 3);
    }

    #[test]
    fn reusing_a_name_with_a_different_kind_is_rejected_not_silently_reused() {
        let (meter, provider) = meter();
        let sink = OpenTelemetryMetricSink::new(meter);
        sink.emit(metric_envelope(
            "requests_total",
            MetricKind::Counter,
            1.0,
            None,
        ))
        .expect("first emit establishes the cached kind");
        let result = sink.emit(metric_envelope(
            "requests_total",
            MetricKind::Histogram,
            1.0,
            None,
        ));
        assert_eq!(result, Err(ObservabilityError::InvalidMetric));
        // Only the original Counter instrument was ever created; the mismatched
        // second call must not silently create or reuse a different instrument.
        assert_eq!(provider.created.lock().expect("created").len(), 1);
    }

    #[test]
    fn unit_on_a_later_call_for_an_already_cached_name_is_not_applied() {
        let (meter, provider) = meter();
        let sink = OpenTelemetryMetricSink::new(meter);
        sink.emit(metric_envelope(
            "latency_ms",
            MetricKind::Histogram,
            1.0,
            Some("ms"),
        ))
        .expect("first emit");
        sink.emit(metric_envelope(
            "latency_ms",
            MetricKind::Histogram,
            2.0,
            Some("seconds"),
        ))
        .expect("second emit reuses the cached instrument regardless of unit");
        // Only one instrument was ever created (the "ms"-unit one); the
        // "seconds" unit on the second call had no effect.
        assert_eq!(provider.created.lock().expect("created").len(), 1);
        assert_eq!(provider.measurements.lock().expect("measurements").len(), 2);
    }

    #[test]
    fn guarantees_are_all_false_except_the_unverifiable_may_block() {
        let (meter, _provider) = meter();
        let sink = OpenTelemetryMetricSink::new(meter);
        assert_eq!(
            sink.guarantees(),
            TelemetryGuarantees {
                durable_across_restart: false,
                visible_across_processes: false,
                delivery_confirmed: false,
                queryable: false,
                may_block: true,
            }
        );
    }

    #[test]
    fn concurrent_first_use_of_the_same_new_name_creates_exactly_one_instrument() {
        let (meter, provider) = meter();
        let sink = Arc::new(OpenTelemetryMetricSink::new(meter));
        let barrier = Arc::new(std::sync::Barrier::new(8));
        let handles: Vec<_> = (0..8)
            .map(|thread_index| {
                let sink = Arc::clone(&sink);
                let barrier = Arc::clone(&barrier);
                std::thread::spawn(move || {
                    barrier.wait();
                    sink.emit(metric_envelope(
                        "concurrent_first_use",
                        MetricKind::Counter,
                        f64::from(thread_index),
                        None,
                    ))
                    .expect("emit");
                })
            })
            .collect();
        for handle in handles {
            handle.join().expect("thread joins");
        }
        // Exactly one instrument was ever created for the racing first-use,
        // and every thread's measurement landed on it: no TOCTOU double
        // creation between the cache check and the insert.
        let created = provider.created.lock().expect("created");
        assert_eq!(
            created.as_slice(),
            &["concurrent_first_use".to_owned()],
            "concurrent first-use must create exactly one instrument, not race a duplicate"
        );
        assert_eq!(provider.measurements.lock().expect("measurements").len(), 8);
    }

    #[derive(Clone, Default)]
    struct PanicOnCreateInstrumentProvider;
    impl InstrumentProvider for PanicOnCreateInstrumentProvider {
        fn f64_counter(&self, _: InstrumentBuilder<'_, Counter<f64>>) -> Counter<f64> {
            panic!("poison the instrument cache mutex for a deterministic failure test")
        }
    }

    #[test]
    fn poisoned_instrument_cache_mutex_fails_closed_as_adapter_failure() {
        // instrument_for holds the cache's MutexGuard for the entire
        // check-then-create-then-insert sequence, so a panic during
        // instrument creation (while the lock is held) poisons the mutex.
        // A subsequent emit call must fail closed as AdapterFailure rather
        // than panicking again or silently recovering.
        let meter = Meter::new(Arc::new(PanicOnCreateInstrumentProvider));
        let sink = Arc::new(OpenTelemetryMetricSink::new(meter));
        let poisoner = Arc::clone(&sink);
        let _ = std::thread::spawn(move || {
            let _ = poisoner.emit(metric_envelope(
                "requests_total",
                MetricKind::Counter,
                1.0,
                None,
            ));
        })
        .join();
        assert_eq!(
            sink.emit(metric_envelope(
                "requests_total",
                MetricKind::Counter,
                1.0,
                None
            )),
            Err(ObservabilityError::AdapterFailure)
        );
    }

    #[test]
    fn a_non_finite_metric_value_never_touches_the_instrument_cache_or_meter() {
        let (meter, provider) = meter();
        let sink = OpenTelemetryMetricSink::new(meter);

        // `MetricEventV1`/`MetricEnvelopeV1` fields are pub, so build a
        // structurally valid-but-semantically-invalid (non-finite value)
        // envelope directly, bypassing the smart-constructor's own
        // `validate_metric` call, to prove the *sink's* independent
        // `validate_metric` call (not just the constructor's) rejects it
        // before ever creating an instrument or calling into the Meter.
        let non_finite = MetricEnvelopeV1 {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            metric: MetricEventV1 {
                name: EventName::new("requests_total").expect("name"),
                kind: MetricKind::Counter,
                value: f64::NAN,
                unit: None,
                attributes: std::collections::BTreeMap::new(),
                timestamp: Timestamp::from_unix_nanos(1),
            },
        };
        let result = sink.emit(non_finite);
        assert_eq!(result, Err(ObservabilityError::InvalidMetric));
        assert!(
            provider.created.lock().expect("created").is_empty(),
            "an invalid metric must never reach the instrument cache or Meter"
        );
        assert!(
            provider
                .measurements
                .lock()
                .expect("measurements")
                .is_empty()
        );
    }
}

mod span_sink {
    use std::collections::BTreeMap;
    use std::future::Future;
    use std::sync::{Arc, Mutex};

    use observability::opentelemetry::OpenTelemetrySpanSink;
    use observability::{
        EventName, EventTarget, ObservabilityError, SpanEnvelopeV1, SpanEventV1, SpanId, SpanSink,
        SpanStatus, TelemetryGuarantees, TenantId, Timestamp, TraceContextV1, TraceFlags, TraceId,
    };
    use opentelemetry_sdk::error::OTelSdkResult;
    use opentelemetry_sdk::trace::SpanData;

    #[derive(Clone, Debug, Default)]
    struct RecordingExporter {
        exported: Arc<Mutex<Vec<Vec<SpanData>>>>,
    }
    impl opentelemetry_sdk::trace::SpanExporter for RecordingExporter {
        fn export(&self, batch: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
            self.exported.lock().expect("exported").push(batch);
            std::future::ready(Ok(()))
        }
    }

    #[derive(Clone, Debug, Default)]
    struct FailingExporter;
    impl opentelemetry_sdk::trace::SpanExporter for FailingExporter {
        fn export(&self, _: Vec<SpanData>) -> impl Future<Output = OTelSdkResult> + Send {
            std::future::ready(Err(
                opentelemetry_sdk::error::OTelSdkError::InternalFailure("export failed".to_owned()),
            ))
        }
    }

    fn runtime() -> tokio::runtime::Runtime {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("multi-thread runtime")
    }

    fn trace_context() -> TraceContextV1 {
        TraceContextV1::new(
            TraceId::new([1u8; 16]).expect("trace id"),
            SpanId::new([2u8; 8]).expect("span id"),
            TraceFlags::from_raw(0x01),
            None,
            BTreeMap::new(),
        )
        .expect("trace context")
    }

    fn span_envelope(status: SpanStatus, attributes: BTreeMap<String, String>) -> SpanEnvelopeV1 {
        SpanEnvelopeV1 {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            span: SpanEventV1::new(
                EventName::new("job.run").expect("name"),
                EventTarget::new("worker").expect("target"),
                trace_context(),
                Some(SpanId::new([3u8; 8]).expect("parent span id")),
                Timestamp::from_unix_nanos(1_000),
                Timestamp::from_unix_nanos(2_000),
                status,
                attributes,
            )
            .expect("span"),
        }
    }

    #[test]
    fn emit_exports_exactly_one_span_preserving_trace_and_span_identity() {
        let runtime = runtime();
        let exporter = RecordingExporter::default();
        let sink = OpenTelemetrySpanSink::new(exporter.clone(), runtime.handle().clone());
        sink.emit(span_envelope(SpanStatus::Ok, BTreeMap::new()))
            .expect("emit");

        let batches = exporter.exported.lock().expect("exported");
        assert_eq!(batches.len(), 1);
        let batch = &batches[0];
        assert_eq!(batch.len(), 1);
        let span = &batch[0];
        assert_eq!(
            span.span_context.trace_id().to_bytes(),
            [1u8; 16],
            "the exported trace_id must be exactly the one this crate's own \
             propagation logic determined, never minted by the exporter"
        );
        assert_eq!(span.span_context.span_id().to_bytes(), [2u8; 8]);
        assert_eq!(span.parent_span_id.to_bytes(), [3u8; 8]);
        assert_eq!(span.name, "job.run");
        assert_eq!(span.status, opentelemetry::trace::Status::Ok);
        assert_eq!(span.span_kind, opentelemetry::trace::SpanKind::Internal);
        assert!(!span.parent_span_is_remote);
        assert_eq!(span.dropped_attributes_count, 0);
        assert!(span.events.is_empty());
        assert!(span.links.is_empty());
    }

    #[test]
    fn tenant_id_and_caller_attributes_are_never_exported() {
        let runtime = runtime();
        let exporter = RecordingExporter::default();
        let sink = OpenTelemetrySpanSink::new(exporter.clone(), runtime.handle().clone());
        sink.emit(span_envelope(
            SpanStatus::Unset,
            BTreeMap::from([("forged".to_owned(), "leaked".to_owned())]),
        ))
        .expect("emit");

        let batches = exporter.exported.lock().expect("exported");
        let span = &batches[0][0];
        assert!(span.attributes.is_empty(), "attributes must never forward");
        let encoded = format!("{span:?}");
        for prohibited in ["tenant", "leaked", "forged"] {
            assert!(!encoded.contains(prohibited), "export leaked {prohibited}");
        }
    }

    #[test]
    fn baggage_is_never_exported() {
        let runtime = runtime();
        let exporter = RecordingExporter::default();
        let sink = OpenTelemetrySpanSink::new(exporter.clone(), runtime.handle().clone());
        let context_with_baggage = TraceContextV1::new(
            TraceId::new([1u8; 16]).expect("trace id"),
            SpanId::new([2u8; 8]).expect("span id"),
            TraceFlags::from_raw(0x01),
            None,
            BTreeMap::from([("secret".to_owned(), "smuggled".to_owned())]),
        )
        .expect("trace context with baggage");
        let envelope = SpanEnvelopeV1 {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            span: SpanEventV1::new(
                EventName::new("job.run").expect("name"),
                EventTarget::new("worker").expect("target"),
                context_with_baggage,
                None,
                Timestamp::from_unix_nanos(1_000),
                Timestamp::from_unix_nanos(2_000),
                SpanStatus::Unset,
                BTreeMap::new(),
            )
            .expect("span"),
        };
        sink.emit(envelope).expect("emit");

        let batches = exporter.exported.lock().expect("exported");
        let encoded = format!("{:?}", batches[0][0]);
        assert!(!encoded.contains("smuggled"), "baggage must never export");
    }

    #[test]
    fn every_span_status_maps_correctly() {
        for (status, expected) in [
            (SpanStatus::Unset, opentelemetry::trace::Status::Unset),
            (SpanStatus::Ok, opentelemetry::trace::Status::Ok),
        ] {
            let runtime = runtime();
            let exporter = RecordingExporter::default();
            let sink = OpenTelemetrySpanSink::new(exporter.clone(), runtime.handle().clone());
            sink.emit(span_envelope(status, BTreeMap::new()))
                .expect("emit");
            let batches = exporter.exported.lock().expect("exported");
            assert_eq!(batches[0][0].status, expected);
        }
        let runtime = runtime();
        let exporter = RecordingExporter::default();
        let sink = OpenTelemetrySpanSink::new(exporter.clone(), runtime.handle().clone());
        sink.emit(span_envelope(SpanStatus::Error, BTreeMap::new()))
            .expect("emit");
        let batches = exporter.exported.lock().expect("exported");
        assert!(matches!(
            batches[0][0].status,
            opentelemetry::trace::Status::Error { .. }
        ));
    }

    #[test]
    fn exporter_failure_is_reported_as_adapter_failure() {
        let runtime = runtime();
        let sink = OpenTelemetrySpanSink::new(FailingExporter, runtime.handle().clone());
        assert_eq!(
            sink.emit(span_envelope(SpanStatus::Unset, BTreeMap::new())),
            Err(ObservabilityError::AdapterFailure)
        );
    }

    #[test]
    fn guarantees_are_all_false_except_may_block() {
        let runtime = runtime();
        let sink =
            OpenTelemetrySpanSink::new(RecordingExporter::default(), runtime.handle().clone());
        assert_eq!(
            sink.guarantees(),
            TelemetryGuarantees {
                durable_across_restart: false,
                visible_across_processes: false,
                delivery_confirmed: false,
                queryable: false,
                may_block: true,
            }
        );
    }

    #[test]
    fn emit_from_a_spawned_blocking_task_does_not_panic() {
        let runtime = runtime();
        let exporter = RecordingExporter::default();
        let sink = Arc::new(OpenTelemetrySpanSink::new(
            exporter.clone(),
            runtime.handle().clone(),
        ));
        runtime.block_on(async {
            let sink = Arc::clone(&sink);
            tokio::task::spawn_blocking(move || {
                sink.emit(span_envelope(SpanStatus::Unset, BTreeMap::new()))
                    .expect("emit from a spawned blocking task must not panic");
            })
            .await
            .expect("spawn_blocking task");
        });
        assert_eq!(exporter.exported.lock().expect("exported").len(), 1);
    }

    #[test]
    fn emit_from_directly_within_an_async_runtime_worker_task_does_not_panic() {
        // The actual adversarial condition block_in_place exists to guard
        // against: emit() called from *directly inside* an async task body
        // running on a runtime worker thread - not via spawn_blocking (which
        // runs on Tokio's separate blocking-thread pool and carries no
        // runtime-context guard, so it never exercises this path). A bare
        // `handle.block_on(...)` called from here panics with "Cannot start
        // a runtime from within a runtime"; block_in_place is what prevents
        // that. This test fails (panics) if block_in_place is ever removed.
        let runtime = runtime();
        let exporter = RecordingExporter::default();
        let sink = Arc::new(OpenTelemetrySpanSink::new(
            exporter.clone(),
            runtime.handle().clone(),
        ));
        runtime.block_on(async {
            // No spawn_blocking here: this closure body runs directly on a
            // scheduler worker thread while executing async work.
            let sink = Arc::clone(&sink);
            tokio::task::yield_now().await;
            sink.emit(span_envelope(SpanStatus::Unset, BTreeMap::new()))
                .expect("emit from directly within an async worker task must not panic");
        });
        assert_eq!(exporter.exported.lock().expect("exported").len(), 1);
    }
}
