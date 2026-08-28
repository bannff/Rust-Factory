//! Framework-free Observability public contract tests.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use observability::{
    Clock, EventName, EventTarget, MAX_ATTRIBUTE_VALUE_BYTES, MAX_ATTRIBUTES, MAX_BODY_BYTES,
    MAX_EVENT_BYTES, MAX_IDENTIFIER_BYTES, MAX_QUERY_LIMIT, ObservabilityError, PublicErrorCode,
    Severity, TelemetryContext, TelemetryEnvelopeV1, TelemetryEventV1, TelemetryGuarantees,
    TelemetryQueryV1, TelemetryReader, TelemetryRecordV1, TelemetryService, TelemetrySink,
    TenantId, Timestamp, record_matches, validate_event, validate_query,
};

fn event_at(timestamp: u64, tenant: &str, body: &str) -> TelemetryRecordV1 {
    TelemetryRecordV1 {
        sequence: timestamp + 1,
        envelope: TelemetryEnvelopeV1 {
            tenant_id: TenantId::new(tenant).expect("tenant"),
            timestamp: Timestamp::from_unix_nanos(timestamp),
            event: TelemetryEventV1::new(
                EventName::new("event").expect("name"),
                EventTarget::new("target").expect("target"),
                Severity::Info,
                body,
                BTreeMap::new(),
            )
            .expect("event"),
        },
    }
}

#[test]
fn identifiers_and_labels_enforce_byte_boundaries_and_grammars() {
    assert!(TenantId::new("a".repeat(MAX_IDENTIFIER_BYTES)).is_ok());
    assert_eq!(
        TenantId::new("a".repeat(MAX_IDENTIFIER_BYTES + 1)),
        Err(ObservabilityError::InvalidId)
    );
    for invalid in ["", "Tenant", "-tenant", "tenant.name", "ténant"] {
        assert_eq!(TenantId::new(invalid), Err(ObservabilityError::InvalidId));
    }

    assert!(EventName::new("a".repeat(MAX_IDENTIFIER_BYTES)).is_ok());
    assert!(EventTarget::new("a".repeat(MAX_IDENTIFIER_BYTES)).is_ok());
    assert_eq!(
        EventName::new("a".repeat(MAX_IDENTIFIER_BYTES + 1)),
        Err(ObservabilityError::InvalidEvent)
    );
    for invalid in [
        "",
        "Uppercase",
        "-leading",
        "line\nbreak",
        "nul\0byte",
        "ténant",
        "/host/path",
    ] {
        assert_eq!(
            EventName::new(invalid),
            Err(ObservabilityError::InvalidEvent)
        );
        assert_eq!(
            EventTarget::new(invalid),
            Err(ObservabilityError::InvalidEvent)
        );
    }
}

#[test]
fn body_attribute_count_key_and_value_byte_boundaries_are_enforced() {
    let exact_body = "é".repeat(MAX_BODY_BYTES / 2);
    assert_eq!(exact_body.len(), MAX_BODY_BYTES);
    assert!(
        TelemetryEventV1::new(
            EventName::new("event").expect("name"),
            EventTarget::new("target").expect("target"),
            Severity::Info,
            exact_body.clone(),
            BTreeMap::new(),
        )
        .is_ok()
    );
    assert_eq!(
        TelemetryEventV1::new(
            EventName::new("event").expect("name"),
            EventTarget::new("target").expect("target"),
            Severity::Info,
            format!("{exact_body}é"),
            BTreeMap::new(),
        ),
        Err(ObservabilityError::LimitExceeded)
    );

    let attributes = (0..MAX_ATTRIBUTES)
        .map(|index| {
            (
                format!("key-{index}"),
                "é".repeat(MAX_ATTRIBUTE_VALUE_BYTES / 2),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let event = TelemetryEventV1::new(
        EventName::new("event").expect("name"),
        EventTarget::new("target").expect("target"),
        Severity::Info,
        "body",
        attributes.clone(),
    )
    .expect("exact attribute ceilings");
    assert_eq!(event.attributes.len(), MAX_ATTRIBUTES);

    let mut too_many = attributes.clone();
    too_many.insert("one-more".to_owned(), "value".to_owned());
    assert_eq!(
        TelemetryEventV1::new(
            EventName::new("event").expect("name"),
            EventTarget::new("target").expect("target"),
            Severity::Info,
            "body",
            too_many,
        ),
        Err(ObservabilityError::LimitExceeded)
    );
    let oversized_value = BTreeMap::from([(
        "key".to_owned(),
        format!("{}é", "a".repeat(MAX_ATTRIBUTE_VALUE_BYTES - 1)),
    )]);
    assert_eq!(
        TelemetryEventV1::new(
            EventName::new("event").expect("name"),
            EventTarget::new("target").expect("target"),
            Severity::Info,
            "body",
            oversized_value,
        ),
        Err(ObservabilityError::LimitExceeded)
    );
    let invalid_key = BTreeMap::from([("bad\nkey".to_owned(), "value".to_owned())]);
    assert_eq!(
        TelemetryEventV1::new(
            EventName::new("event").expect("name"),
            EventTarget::new("target").expect("target"),
            Severity::Info,
            "body",
            invalid_key,
        ),
        Err(ObservabilityError::InvalidEvent)
    );
}

#[test]
fn maximum_legal_event_is_bounded_but_aggregate_ceiling_is_currently_not_tight() {
    let attributes = (0..MAX_ATTRIBUTES)
        .map(|index| {
            let suffix = index.to_string();
            (
                format!(
                    "{}{}",
                    "k".repeat(MAX_IDENTIFIER_BYTES - suffix.len()),
                    suffix
                ),
                "v".repeat(MAX_ATTRIBUTE_VALUE_BYTES),
            )
        })
        .collect::<BTreeMap<_, _>>();
    let event = TelemetryEventV1::new(
        EventName::new("n".repeat(MAX_IDENTIFIER_BYTES)).expect("name"),
        EventTarget::new("t".repeat(MAX_IDENTIFIER_BYTES)).expect("target"),
        Severity::Error,
        "b".repeat(MAX_BODY_BYTES),
        attributes,
    )
    .expect("all individual ceilings remain under aggregate ceiling");
    assert!(validate_event(&event).is_ok());
    let maximum_reachable = 2 * MAX_IDENTIFIER_BYTES
        + MAX_BODY_BYTES
        + MAX_ATTRIBUTES * (MAX_IDENTIFIER_BYTES + MAX_ATTRIBUTE_VALUE_BYTES);
    assert!(maximum_reachable < MAX_EVENT_BYTES);
}

#[test]
fn query_limits_and_half_open_windows_are_exact() {
    assert_eq!(
        TelemetryQueryV1::new(0),
        Err(ObservabilityError::InvalidQuery)
    );
    assert!(TelemetryQueryV1::new(MAX_QUERY_LIMIT).is_ok());
    assert_eq!(
        TelemetryQueryV1::new(MAX_QUERY_LIMIT + 1),
        Err(ObservabilityError::LimitExceeded)
    );

    let mut query = TelemetryQueryV1::new(1).expect("query");
    query.since = Some(Timestamp::from_unix_nanos(10));
    query.until = Some(Timestamp::from_unix_nanos(20));
    assert!(record_matches(&event_at(10, "tenant", "since"), &query));
    assert!(record_matches(&event_at(19, "tenant", "inside"), &query));
    assert!(!record_matches(&event_at(20, "tenant", "until"), &query));

    query.until = query.since;
    assert_eq!(
        validate_query(&query, MAX_QUERY_LIMIT),
        Err(ObservabilityError::InvalidQuery)
    );
    query.until = Some(Timestamp::from_unix_nanos(9));
    assert_eq!(
        validate_query(&query, MAX_QUERY_LIMIT),
        Err(ObservabilityError::InvalidQuery)
    );
}

#[test]
fn errors_have_stable_nonleaking_display_debug_and_public_codes() {
    for (error, code, debug) in [
        (
            ObservabilityError::InvalidId,
            PublicErrorCode::InvalidId,
            "InvalidId",
        ),
        (
            ObservabilityError::InvalidEvent,
            PublicErrorCode::InvalidEvent,
            "InvalidEvent",
        ),
        (
            ObservabilityError::InvalidQuery,
            PublicErrorCode::InvalidQuery,
            "InvalidQuery",
        ),
        (
            ObservabilityError::InvalidConfiguration,
            PublicErrorCode::InvalidConfiguration,
            "InvalidConfiguration",
        ),
        (
            ObservabilityError::LimitExceeded,
            PublicErrorCode::LimitExceeded,
            "LimitExceeded",
        ),
        (
            ObservabilityError::AdapterFailure,
            PublicErrorCode::OperationFailed,
            "OperationFailed",
        ),
    ] {
        assert_eq!(error.public_code(), code);
        assert_eq!(format!("{error:?}"), debug);
        assert_eq!(
            error.to_string(),
            format!("observability operation failed: {debug}")
        );
        assert!(!error.to_string().contains("AdapterFailure"));
    }
}

#[derive(Clone)]
struct FixedClock(Timestamp);
impl Clock for FixedClock {
    fn now(&self) -> Result<Timestamp, ObservabilityError> {
        Ok(self.0)
    }
}

#[derive(Clone, Default)]
struct CaptureSink(Arc<Mutex<Vec<TelemetryEnvelopeV1>>>);
impl TelemetrySink for CaptureSink {
    fn emit(&self, envelope: TelemetryEnvelopeV1) -> Result<(), ObservabilityError> {
        self.0.lock().expect("capture").push(envelope);
        Ok(())
    }
    fn guarantees(&self) -> TelemetryGuarantees {
        TelemetryGuarantees {
            durable_across_restart: true,
            visible_across_processes: false,
            delivery_confirmed: true,
            queryable: false,
        }
    }
}

#[derive(Clone)]
struct HostileReader {
    records: Vec<TelemetryRecordV1>,
    fail: bool,
}
impl TelemetryReader for HostileReader {
    fn query(
        &self,
        _: &TenantId,
        _: &TelemetryQueryV1,
    ) -> Result<Vec<TelemetryRecordV1>, ObservabilityError> {
        if self.fail {
            Err(ObservabilityError::AdapterFailure)
        } else {
            Ok(self.records.clone())
        }
    }
    fn guarantees(&self) -> TelemetryGuarantees {
        TelemetryGuarantees {
            durable_across_restart: true,
            visible_across_processes: true,
            delivery_confirmed: false,
            queryable: true,
        }
    }
}

struct FailingSink;
impl TelemetrySink for FailingSink {
    fn emit(&self, _: TelemetryEnvelopeV1) -> Result<(), ObservabilityError> {
        Err(ObservabilityError::AdapterFailure)
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

#[test]
fn service_stamps_trusted_tenant_and_clock_and_propagates_sink_failure() {
    let sink = CaptureSink::default();
    let service = TelemetryService::new(
        sink.clone(),
        HostileReader {
            records: vec![],
            fail: false,
        },
        FixedClock(Timestamp::from_unix_nanos(42)),
        4,
    )
    .expect("service");
    let context = TelemetryContext::new(TenantId::new("trusted").expect("tenant"));
    service
        .emit(&context, event_at(1, "ignored", "body").envelope.event)
        .expect("emit");
    let captured = sink.0.lock().expect("capture");
    assert_eq!(captured[0].tenant_id.as_str(), "trusted");
    assert_eq!(captured[0].timestamp.as_unix_nanos(), 42);

    let failing = TelemetryService::new(
        FailingSink,
        HostileReader {
            records: vec![],
            fail: false,
        },
        FixedClock(Timestamp::from_unix_nanos(1)),
        1,
    )
    .expect("service");
    assert_eq!(
        failing.emit(&context, event_at(1, "ignored", "body").envelope.event),
        Err(ObservabilityError::AdapterFailure)
    );
}

#[test]
fn service_revalidates_hostile_results_filters_tenant_and_query_then_truncates_matches() {
    let context = TelemetryContext::new(TenantId::new("tenant").expect("tenant"));
    let mut invalid = event_at(40, "tenant", "invalid-sequence");
    invalid.sequence = 0;
    let records = vec![
        event_at(50, "foreign", "foreign"),
        invalid,
        event_at(30, "tenant", "match-1"),
        event_at(20, "tenant", "too-early"),
        event_at(29, "tenant", "match-2"),
        event_at(28, "tenant", "match-3"),
    ];
    let service = TelemetryService::new(
        CaptureSink::default(),
        HostileReader {
            records,
            fail: false,
        },
        FixedClock(Timestamp::from_unix_nanos(1)),
        2,
    )
    .expect("service");
    let mut query = TelemetryQueryV1::new(2).expect("query");
    query.since = Some(Timestamp::from_unix_nanos(25));
    let found = service.query(&context, &query).expect("query");
    assert_eq!(
        found
            .iter()
            .map(|record| record.envelope.event.body.as_str())
            .collect::<Vec<_>>(),
        ["match-1", "match-2"]
    );

    let narrowed = TelemetryService::new(
        CaptureSink::default(),
        HostileReader {
            records: vec![],
            fail: false,
        },
        FixedClock(Timestamp::from_unix_nanos(1)),
        1,
    )
    .expect("service");
    assert_eq!(
        narrowed.query(&context, &TelemetryQueryV1::new(2).expect("query")),
        Err(ObservabilityError::LimitExceeded)
    );
    let failed = TelemetryService::new(
        CaptureSink::default(),
        HostileReader {
            records: vec![],
            fail: true,
        },
        FixedClock(Timestamp::from_unix_nanos(1)),
        1,
    )
    .expect("service");
    assert_eq!(
        failed.query(&context, &TelemetryQueryV1::new(1).expect("query")),
        Err(ObservabilityError::AdapterFailure)
    );
}

#[test]
fn ports_are_object_safe_and_arc_dyn_polymorphic() {
    let sink: Arc<dyn TelemetrySink> = Arc::new(CaptureSink::default());
    let reader: Arc<dyn TelemetryReader> = Arc::new(HostileReader {
        records: vec![],
        fail: false,
    });
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(Timestamp::from_unix_nanos(7)));
    let service = TelemetryService::new(sink, reader, clock, 1).expect("trait object service");
    let context = TelemetryContext::new(TenantId::new("tenant").expect("tenant"));
    service
        .emit(&context, event_at(1, "ignored", "body").envelope.event)
        .expect("emit through Arc<dyn>");
    assert!(
        service
            .query(&context, &TelemetryQueryV1::new(1).expect("query"))
            .expect("query")
            .is_empty()
    );
    assert_eq!(
        service.guarantees(),
        TelemetryGuarantees {
            durable_across_restart: true,
            visible_across_processes: false,
            delivery_confirmed: true,
            queryable: true
        }
    );
}
