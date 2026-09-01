//! Framework-free Observability public contract tests.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use observability::{
    Clock, EventName, EventTarget, MAX_ATTRIBUTE_VALUE_BYTES, MAX_ATTRIBUTES, MAX_BAGGAGE_ENTRIES,
    MAX_BAGGAGE_VALUE_BYTES, MAX_BODY_BYTES, MAX_EVENT_BYTES, MAX_IDENTIFIER_BYTES,
    MAX_METRIC_BYTES, MAX_METRIC_UNIT_BYTES, MAX_QUERY_LIMIT, MAX_SPAN_BYTES, MAX_TRACESTATE_BYTES,
    MetricEnvelopeV1, MetricEventV1, MetricKind, MetricSink, ObservabilityError, PublicErrorCode,
    Severity, SpanEnvelopeV1, SpanEventV1, SpanId, SpanSink, SpanStatus, TelemetryContext,
    TelemetryEnvelopeV1, TelemetryEventV1, TelemetryGuarantees, TelemetryQueryV1, TelemetryReader,
    TelemetryRecordV1, TelemetryService, TelemetrySink, TenantId, Timestamp, TraceContextV1,
    TraceFlags, TraceId, record_matches, validate_event, validate_metric, validate_query,
    validate_span, validate_trace_context,
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
            may_block: false,
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
            may_block: false,
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
            may_block: false,
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
            queryable: true,
            may_block: false,
        }
    );
}

/// A sink/reader pair used solely to prove the deliberate asymmetry in
/// `TelemetryService::guarantees().may_block`: it mirrors the sink and
/// ignores the reader, unlike `durable_across_restart`/
/// `visible_across_processes`, which AND both sides.
struct MayBlockSink(bool);
impl TelemetrySink for MayBlockSink {
    fn emit(&self, _: TelemetryEnvelopeV1) -> Result<(), ObservabilityError> {
        Ok(())
    }
    fn guarantees(&self) -> TelemetryGuarantees {
        TelemetryGuarantees {
            durable_across_restart: false,
            visible_across_processes: false,
            delivery_confirmed: false,
            queryable: false,
            may_block: self.0,
        }
    }
}
struct MayBlockReader(bool);
impl TelemetryReader for MayBlockReader {
    fn query(
        &self,
        _: &TenantId,
        _: &TelemetryQueryV1,
    ) -> Result<Vec<TelemetryRecordV1>, ObservabilityError> {
        Ok(vec![])
    }
    fn guarantees(&self) -> TelemetryGuarantees {
        TelemetryGuarantees {
            durable_across_restart: false,
            visible_across_processes: false,
            delivery_confirmed: false,
            queryable: false,
            may_block: self.0,
        }
    }
}

#[test]
fn service_guarantees_may_block_mirrors_the_sink_and_ignores_a_differing_reader() {
    let clock: Arc<dyn Clock> = Arc::new(FixedClock(Timestamp::from_unix_nanos(1)));

    let blocking_sink_nonblocking_reader =
        TelemetryService::new(MayBlockSink(true), MayBlockReader(false), clock.clone(), 1)
            .expect("service");
    assert!(blocking_sink_nonblocking_reader.guarantees().may_block);

    let nonblocking_sink_blocking_reader =
        TelemetryService::new(MayBlockSink(false), MayBlockReader(true), clock, 1)
            .expect("service");
    assert!(!nonblocking_sink_blocking_reader.guarantees().may_block);
}

fn sample_trace_context() -> TraceContextV1 {
    TraceContextV1::new(
        TraceId::new([1u8; 16]).expect("trace id"),
        SpanId::new([2u8; 8]).expect("span id"),
        TraceFlags::from_raw(0x01),
        None,
        BTreeMap::new(),
    )
    .expect("trace context")
}

#[test]
fn trace_and_span_ids_reject_all_zero_bytes() {
    assert_eq!(
        TraceId::new([0u8; 16]),
        Err(ObservabilityError::InvalidTraceContext)
    );
    assert!(TraceId::new([1u8; 16]).is_ok());
    assert_eq!(
        SpanId::new([0u8; 8]),
        Err(ObservabilityError::InvalidTraceContext)
    );
    assert!(SpanId::new([1u8; 8]).is_ok());
}

#[test]
fn trace_flags_expose_the_sampled_bit_and_preserve_the_raw_byte() {
    assert!(!TraceFlags::from_raw(0x00).is_sampled());
    assert!(TraceFlags::from_raw(0x01).is_sampled());
    assert_eq!(TraceFlags::from_raw(0xFF).as_raw(), 0xFF);
}

#[test]
fn trace_context_enforces_tracestate_and_baggage_boundaries() {
    let trace_id = TraceId::new([1u8; 16]).expect("trace id");
    let span_id = SpanId::new([2u8; 8]).expect("span id");
    let flags = TraceFlags::from_raw(0x01);

    assert!(
        TraceContextV1::new(
            trace_id,
            span_id,
            flags,
            Some("x".repeat(MAX_TRACESTATE_BYTES)),
            BTreeMap::new(),
        )
        .is_ok()
    );
    assert_eq!(
        TraceContextV1::new(
            trace_id,
            span_id,
            flags,
            Some("x".repeat(MAX_TRACESTATE_BYTES + 1)),
            BTreeMap::new(),
        ),
        Err(ObservabilityError::LimitExceeded)
    );

    let exact_baggage = (0..MAX_BAGGAGE_ENTRIES)
        .map(|index| (format!("key-{index}"), "v".repeat(MAX_BAGGAGE_VALUE_BYTES)))
        .collect::<BTreeMap<_, _>>();
    assert!(TraceContextV1::new(trace_id, span_id, flags, None, exact_baggage.clone()).is_ok());
    let mut too_many = exact_baggage.clone();
    too_many.insert("one-more".to_owned(), "v".to_owned());
    assert_eq!(
        TraceContextV1::new(trace_id, span_id, flags, None, too_many),
        Err(ObservabilityError::LimitExceeded)
    );
    let oversized_value =
        BTreeMap::from([("key".to_owned(), "v".repeat(MAX_BAGGAGE_VALUE_BYTES + 1))]);
    assert_eq!(
        TraceContextV1::new(trace_id, span_id, flags, None, oversized_value),
        Err(ObservabilityError::LimitExceeded)
    );
    let invalid_key = BTreeMap::from([("Bad Key".to_owned(), "v".to_owned())]);
    assert_eq!(
        TraceContextV1::new(trace_id, span_id, flags, None, invalid_key),
        Err(ObservabilityError::InvalidTraceContext)
    );
    assert!(validate_trace_context(&sample_trace_context()).is_ok());
}

#[test]
fn span_events_enforce_ordering_bounds_and_aggregate_ceiling() {
    let context = sample_trace_context();
    let start = Timestamp::from_unix_nanos(10);
    let end = Timestamp::from_unix_nanos(20);
    let span = SpanEventV1::new(
        EventName::new("span").expect("name"),
        EventTarget::new("target").expect("target"),
        context.clone(),
        None,
        start,
        end,
        SpanStatus::Ok,
        BTreeMap::new(),
    )
    .expect("valid span");
    assert!(validate_span(&span).is_ok());

    assert_eq!(
        SpanEventV1::new(
            EventName::new("span").expect("name"),
            EventTarget::new("target").expect("target"),
            context.clone(),
            None,
            end,
            start,
            SpanStatus::Ok,
            BTreeMap::new(),
        ),
        Err(ObservabilityError::InvalidSpan)
    );

    let oversized_attributes =
        BTreeMap::from([("key".to_owned(), "v".repeat(MAX_ATTRIBUTE_VALUE_BYTES + 1))]);
    assert_eq!(
        SpanEventV1::new(
            EventName::new("span").expect("name"),
            EventTarget::new("target").expect("target"),
            context.clone(),
            None,
            start,
            end,
            SpanStatus::Error,
            oversized_attributes,
        ),
        Err(ObservabilityError::LimitExceeded)
    );

    let too_many_attributes = (0..=MAX_ATTRIBUTES)
        .map(|index| (format!("key-{index}"), "v".to_owned()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        SpanEventV1::new(
            EventName::new("span").expect("name"),
            EventTarget::new("target").expect("target"),
            context,
            None,
            start,
            end,
            SpanStatus::Unset,
            too_many_attributes,
        ),
        Err(ObservabilityError::LimitExceeded)
    );

    let maximum_reachable = 2 * MAX_IDENTIFIER_BYTES
        + MAX_ATTRIBUTES * (MAX_IDENTIFIER_BYTES + MAX_ATTRIBUTE_VALUE_BYTES);
    assert!(maximum_reachable < MAX_SPAN_BYTES);
}

#[test]
fn metrics_enforce_finite_values_units_and_aggregate_ceiling() {
    let timestamp = Timestamp::from_unix_nanos(1);
    let metric = MetricEventV1::new(
        EventName::new("requests_total").expect("name"),
        MetricKind::Counter,
        1.0,
        Some("count".to_owned()),
        BTreeMap::new(),
        timestamp,
    )
    .expect("valid metric");
    assert!(validate_metric(&metric).is_ok());

    for invalid in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert_eq!(
            MetricEventV1::new(
                EventName::new("requests_total").expect("name"),
                MetricKind::Gauge,
                invalid,
                None,
                BTreeMap::new(),
                timestamp,
            ),
            Err(ObservabilityError::InvalidMetric)
        );
    }

    assert_eq!(
        MetricEventV1::new(
            EventName::new("requests_total").expect("name"),
            MetricKind::Histogram,
            1.0,
            Some("u".repeat(MAX_METRIC_UNIT_BYTES + 1)),
            BTreeMap::new(),
            timestamp,
        ),
        Err(ObservabilityError::LimitExceeded)
    );

    let too_many_attributes = (0..=MAX_ATTRIBUTES)
        .map(|index| (format!("key-{index}"), "v".to_owned()))
        .collect::<BTreeMap<_, _>>();
    assert_eq!(
        MetricEventV1::new(
            EventName::new("requests_total").expect("name"),
            MetricKind::Counter,
            1.0,
            None,
            too_many_attributes,
            timestamp,
        ),
        Err(ObservabilityError::LimitExceeded)
    );

    // Unlike span/event, per-field ceilings alone can exceed MAX_METRIC_BYTES,
    // so the aggregate size check in validate_metric is load-bearing, not
    // redundant with the individual field ceilings.
    let maximum_reachable = MAX_IDENTIFIER_BYTES
        + MAX_METRIC_UNIT_BYTES
        + MAX_ATTRIBUTES * (MAX_IDENTIFIER_BYTES + MAX_ATTRIBUTE_VALUE_BYTES);
    assert!(maximum_reachable > MAX_METRIC_BYTES);
    let maximal_attributes = (0..MAX_ATTRIBUTES)
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
    assert_eq!(
        MetricEventV1::new(
            EventName::new("n".repeat(MAX_IDENTIFIER_BYTES)).expect("name"),
            MetricKind::Counter,
            1.0,
            Some("u".repeat(MAX_METRIC_UNIT_BYTES)),
            maximal_attributes,
            timestamp,
        ),
        Err(ObservabilityError::LimitExceeded)
    );
}

#[derive(Clone, Default)]
struct CaptureSpanSink(Arc<Mutex<Vec<SpanEnvelopeV1>>>);
impl SpanSink for CaptureSpanSink {
    fn emit(&self, envelope: SpanEnvelopeV1) -> Result<(), ObservabilityError> {
        self.0.lock().expect("capture").push(envelope);
        Ok(())
    }
    fn guarantees(&self) -> TelemetryGuarantees {
        TelemetryGuarantees {
            durable_across_restart: false,
            visible_across_processes: false,
            delivery_confirmed: false,
            queryable: false,
            may_block: false,
        }
    }
}

#[derive(Clone, Default)]
struct CaptureMetricSink(Arc<Mutex<Vec<MetricEnvelopeV1>>>);
impl MetricSink for CaptureMetricSink {
    fn emit(&self, envelope: MetricEnvelopeV1) -> Result<(), ObservabilityError> {
        self.0.lock().expect("capture").push(envelope);
        Ok(())
    }
    fn guarantees(&self) -> TelemetryGuarantees {
        TelemetryGuarantees {
            durable_across_restart: false,
            visible_across_processes: false,
            delivery_confirmed: false,
            queryable: false,
            may_block: false,
        }
    }
}

#[test]
fn span_and_metric_sink_ports_are_object_safe_and_arc_dyn_polymorphic() {
    let span_sink: Arc<dyn SpanSink> = Arc::new(CaptureSpanSink::default());
    let metric_sink: Arc<dyn MetricSink> = Arc::new(CaptureMetricSink::default());

    let span = SpanEventV1::new(
        EventName::new("span").expect("name"),
        EventTarget::new("target").expect("target"),
        sample_trace_context(),
        None,
        Timestamp::from_unix_nanos(1),
        Timestamp::from_unix_nanos(2),
        SpanStatus::Ok,
        BTreeMap::new(),
    )
    .expect("span");
    span_sink
        .emit(SpanEnvelopeV1 {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            span,
        })
        .expect("emit span through Arc<dyn>");

    let metric = MetricEventV1::new(
        EventName::new("requests_total").expect("name"),
        MetricKind::Counter,
        1.0,
        None,
        BTreeMap::new(),
        Timestamp::from_unix_nanos(1),
    )
    .expect("metric");
    metric_sink
        .emit(MetricEnvelopeV1 {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            metric,
        })
        .expect("emit metric through Arc<dyn>");
}

#[test]
fn trace_span_and_metric_errors_have_stable_nonleaking_codes() {
    for (error, code, debug) in [
        (
            ObservabilityError::InvalidTraceContext,
            PublicErrorCode::InvalidTraceContext,
            "InvalidTraceContext",
        ),
        (
            ObservabilityError::InvalidSpan,
            PublicErrorCode::InvalidSpan,
            "InvalidSpan",
        ),
        (
            ObservabilityError::InvalidMetric,
            PublicErrorCode::InvalidMetric,
            "InvalidMetric",
        ),
    ] {
        assert_eq!(error.public_code(), code);
        assert_eq!(format!("{error:?}"), debug);
        assert_eq!(
            error.to_string(),
            format!("observability operation failed: {debug}")
        );
    }
}
