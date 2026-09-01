use std::collections::BTreeMap;

use crate::{ObservabilityError, validation};

pub const MAX_IDENTIFIER_BYTES: usize = 128;
pub const MAX_BODY_BYTES: usize = 16 * 1024;
pub const MAX_ATTRIBUTES: usize = 32;
pub const MAX_ATTRIBUTE_VALUE_BYTES: usize = 1024;
pub const MAX_EVENT_BYTES: usize = 64 * 1024;
pub const MAX_QUERY_LIMIT: usize = 256;
pub const MAX_LOCAL_EVENTS_PER_TENANT: usize = 4096;
pub const MAX_LOCAL_EVENTS_TOTAL: usize = 4096;

macro_rules! bounded_name {
    ($name:ident, $validator:ident) => {
        #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ObservabilityError> {
                let value = value.into();
                validation::$validator(&value)?;
                Ok(Self(value))
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

bounded_name!(TenantId, validate_tenant_id);
bounded_name!(EventName, validate_event_name);
bounded_name!(EventTarget, validate_event_target);

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct Timestamp(u64);
impl Timestamp {
    #[must_use]
    pub const fn from_unix_nanos(value: u64) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_unix_nanos(self) -> u64 {
        self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Severity {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelemetryEventV1 {
    pub name: EventName,
    pub target: EventTarget,
    pub severity: Severity,
    pub body: String,
    pub attributes: BTreeMap<String, String>,
}
impl TelemetryEventV1 {
    pub fn new(
        name: EventName,
        target: EventTarget,
        severity: Severity,
        body: impl Into<String>,
        attributes: BTreeMap<String, String>,
    ) -> Result<Self, ObservabilityError> {
        let event = Self {
            name,
            target,
            severity,
            body: body.into(),
            attributes,
        };
        validation::validate_event(&event)?;
        Ok(event)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelemetryEnvelopeV1 {
    pub tenant_id: TenantId,
    pub timestamp: Timestamp,
    pub event: TelemetryEventV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelemetryRecordV1 {
    pub sequence: u64,
    pub envelope: TelemetryEnvelopeV1,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelemetryQueryV1 {
    pub since: Option<Timestamp>,
    pub until: Option<Timestamp>,
    pub minimum_severity: Option<Severity>,
    pub event_name: Option<EventName>,
    pub target: Option<EventTarget>,
    pub limit: usize,
}
impl TelemetryQueryV1 {
    pub fn new(limit: usize) -> Result<Self, ObservabilityError> {
        let query = Self {
            since: None,
            until: None,
            minimum_severity: None,
            event_name: None,
            target: None,
            limit,
        };
        validation::validate_query(&query, MAX_QUERY_LIMIT)?;
        Ok(query)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TelemetryContext {
    tenant_id: TenantId,
}
impl TelemetryContext {
    #[must_use]
    pub const fn new(tenant_id: TenantId) -> Self {
        Self { tenant_id }
    }

    #[must_use]
    pub const fn tenant_id(&self) -> &TenantId {
        &self.tenant_id
    }
}

/// Truthful, not aspirational: every field states what an adapter has
/// verified about its own behavior, never what it merely hopes or intends.
/// An adapter that cannot verify a property SHALL report the conservative
/// (safety-losing) value rather than the optimistic one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct TelemetryGuarantees {
    pub durable_across_restart: bool,
    pub visible_across_processes: bool,
    pub delivery_confirmed: bool,
    pub queryable: bool,
    /// `true` if calling `emit` may perform network or disk I/O, acquire a
    /// lock that could be held during such I/O, or otherwise block for an
    /// unbounded time. An adapter that wraps an injected downstream
    /// dependency (for example an OTEL `Logger`/`Meter` constructed and
    /// owned by a composition root) cannot observe or bound that
    /// dependency's I/O behavior, and SHALL report `true` in that case:
    /// "unknown" is not a safe substitute for "non-blocking" for a caller
    /// deciding whether it is safe to invoke `emit` synchronously from a
    /// hot path. This field describes the emit path only; it says nothing
    /// about `query`.
    pub may_block: bool,
}

// --- Trace/span model ---
//
// `TraceContextV1` is deliberately a distinct type from `policy::CorrelationId`
// (an orthogonal, Factory-native business-correlation identifier): the W3C
// trace_id/span_id pair is a wire-format-shaped identity for cross-process
// propagation, not a redefinition of Factory's own correlation concept. A
// caller that wants both should carry a `CorrelationId` and a
// `TraceContextV1` side by side.

pub const MAX_TRACESTATE_BYTES: usize = 512;
pub const MAX_BAGGAGE_ENTRIES: usize = 16;
pub const MAX_BAGGAGE_VALUE_BYTES: usize = 256;
pub const MAX_SPAN_BYTES: usize = 64 * 1024;

/// A W3C `trace-id`: exactly 16 bytes, never all-zero.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TraceId([u8; 16]);
impl TraceId {
    pub fn new(bytes: [u8; 16]) -> Result<Self, ObservabilityError> {
        (bytes != [0u8; 16])
            .then_some(Self(bytes))
            .ok_or(ObservabilityError::InvalidTraceContext)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 16] {
        &self.0
    }
}

/// A W3C `parent-id` (span id): exactly 8 bytes, never all-zero.
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct SpanId([u8; 8]);
impl SpanId {
    pub fn new(bytes: [u8; 8]) -> Result<Self, ObservabilityError> {
        (bytes != [0u8; 8])
            .then_some(Self(bytes))
            .ok_or(ObservabilityError::InvalidTraceContext)
    }

    #[must_use]
    pub const fn as_bytes(&self) -> &[u8; 8] {
        &self.0
    }
}

/// The W3C `trace-flags` byte. Only bit 0 (`sampled`) is currently defined by
/// the W3C spec; the raw byte is preserved and forwarded unchanged.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TraceFlags(u8);
impl TraceFlags {
    #[must_use]
    pub const fn from_raw(value: u8) -> Self {
        Self(value)
    }

    #[must_use]
    pub const fn as_raw(self) -> u8 {
        self.0
    }

    #[must_use]
    pub const fn is_sampled(self) -> bool {
        self.0 & 0x01 != 0
    }
}

/// A propagated W3C trace context: `traceparent` (`trace_id`/`span_id`/flags),
/// an opaque bounded `tracestate` passthrough, and bounded `baggage`.
/// `tracestate` is stored and forwarded as an opaque string because its
/// contents are vendor-specific per the W3C spec; only its byte length is
/// bounded here, never its internal structure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TraceContextV1 {
    pub trace_id: TraceId,
    pub span_id: SpanId,
    pub trace_flags: TraceFlags,
    pub trace_state: Option<String>,
    pub baggage: BTreeMap<String, String>,
}
impl TraceContextV1 {
    pub fn new(
        trace_id: TraceId,
        span_id: SpanId,
        trace_flags: TraceFlags,
        trace_state: Option<String>,
        baggage: BTreeMap<String, String>,
    ) -> Result<Self, ObservabilityError> {
        let context = Self {
            trace_id,
            span_id,
            trace_flags,
            trace_state,
            baggage,
        };
        validation::validate_trace_context(&context)?;
        Ok(context)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SpanStatus {
    Unset,
    Ok,
    Error,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpanEventV1 {
    pub name: EventName,
    pub target: EventTarget,
    pub context: TraceContextV1,
    pub parent_span_id: Option<SpanId>,
    pub start: Timestamp,
    pub end: Timestamp,
    pub status: SpanStatus,
    pub attributes: BTreeMap<String, String>,
}
impl SpanEventV1 {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: EventName,
        target: EventTarget,
        context: TraceContextV1,
        parent_span_id: Option<SpanId>,
        start: Timestamp,
        end: Timestamp,
        status: SpanStatus,
        attributes: BTreeMap<String, String>,
    ) -> Result<Self, ObservabilityError> {
        let span = Self {
            name,
            target,
            context,
            parent_span_id,
            start,
            end,
            status,
            attributes,
        };
        validation::validate_span(&span)?;
        Ok(span)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SpanEnvelopeV1 {
    pub tenant_id: TenantId,
    pub span: SpanEventV1,
}

// --- Metric model ---

pub const MAX_METRIC_UNIT_BYTES: usize = 32;
pub const MAX_METRIC_BYTES: usize = 16 * 1024;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricKind {
    Counter,
    Gauge,
    Histogram,
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricEventV1 {
    pub name: EventName,
    pub kind: MetricKind,
    pub value: f64,
    pub unit: Option<String>,
    pub attributes: BTreeMap<String, String>,
    pub timestamp: Timestamp,
}
impl MetricEventV1 {
    pub fn new(
        name: EventName,
        kind: MetricKind,
        value: f64,
        unit: Option<String>,
        attributes: BTreeMap<String, String>,
        timestamp: Timestamp,
    ) -> Result<Self, ObservabilityError> {
        let metric = Self {
            name,
            kind,
            value,
            unit,
            attributes,
            timestamp,
        };
        validation::validate_metric(&metric)?;
        Ok(metric)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct MetricEnvelopeV1 {
    pub tenant_id: TenantId,
    pub metric: MetricEventV1,
}
