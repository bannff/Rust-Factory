#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]

//! Bounded, tenant-scoped operational log telemetry.

mod error;
mod model;
mod port;
mod propagation;
mod service;
mod validation;

#[cfg(feature = "local")]
pub mod local;
#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "opentelemetry")]
pub mod opentelemetry;
#[cfg(feature = "settings")]
pub mod settings;

pub use error::{ObservabilityError, PublicErrorCode};
pub use model::{
    EventName, EventTarget, MAX_ATTRIBUTE_VALUE_BYTES, MAX_ATTRIBUTES, MAX_BAGGAGE_ENTRIES,
    MAX_BAGGAGE_VALUE_BYTES, MAX_BODY_BYTES, MAX_EVENT_BYTES, MAX_IDENTIFIER_BYTES,
    MAX_LOCAL_EVENTS_PER_TENANT, MAX_LOCAL_EVENTS_TOTAL, MAX_METRIC_BYTES, MAX_METRIC_UNIT_BYTES,
    MAX_QUERY_LIMIT, MAX_SPAN_BYTES, MAX_TRACESTATE_BYTES, MetricEnvelopeV1, MetricEventV1,
    MetricKind, Severity, SpanEnvelopeV1, SpanEventV1, SpanId, SpanStatus, TelemetryContext,
    TelemetryEnvelopeV1, TelemetryEventV1, TelemetryGuarantees, TelemetryQueryV1,
    TelemetryRecordV1, TenantId, Timestamp, TraceContextV1, TraceFlags, TraceId,
};
pub use port::{Clock, MetricSink, SpanSink, TelemetryReader, TelemetrySink};
pub use propagation::{
    extract, format_baggage, format_traceparent, inject, parse_baggage, parse_trace_state,
    parse_traceparent,
};
pub use service::TelemetryService;
pub use validation::{
    record_matches, validate_envelope, validate_event, validate_metric, validate_query,
    validate_record, validate_span, validate_tenant_id, validate_trace_context,
};
