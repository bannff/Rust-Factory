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
            queryable: false
        }
    );
}
