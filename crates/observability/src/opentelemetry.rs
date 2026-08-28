use std::time::{Duration, UNIX_EPOCH};

use opentelemetry::logs::{LogRecord, Logger, Severity as OtelSeverity};

use crate::{
    ObservabilityError, Severity, TelemetryEnvelopeV1, TelemetryGuarantees, TelemetrySink,
    validate_envelope,
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
