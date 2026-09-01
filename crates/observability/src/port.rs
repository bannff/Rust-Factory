use std::sync::Arc;

use crate::{
    MetricEnvelopeV1, ObservabilityError, SpanEnvelopeV1, TelemetryEnvelopeV1, TelemetryGuarantees,
    TelemetryQueryV1, TelemetryRecordV1, TenantId, Timestamp,
};

pub trait TelemetrySink: Send + Sync {
    fn emit(&self, envelope: TelemetryEnvelopeV1) -> Result<(), ObservabilityError>;
    fn guarantees(&self) -> TelemetryGuarantees;
}

pub trait TelemetryReader: Send + Sync {
    fn query(
        &self,
        tenant_id: &TenantId,
        query: &TelemetryQueryV1,
    ) -> Result<Vec<TelemetryRecordV1>, ObservabilityError>;
    fn guarantees(&self) -> TelemetryGuarantees;
}

pub trait Clock: Send + Sync {
    fn now(&self) -> Result<Timestamp, ObservabilityError>;
}

/// Emits one completed span. Deliberately separate from [`TelemetrySink`]:
/// spans are a distinct OTEL signal type from logs, with their own shape
/// (start/end, parent/child, W3C trace context) rather than a single
/// discrete event. An adapter may back both traits with the same underlying
/// OTEL SDK/exporter, but the port surface stays additive and untangled.
pub trait SpanSink: Send + Sync {
    fn emit(&self, envelope: SpanEnvelopeV1) -> Result<(), ObservabilityError>;
    fn guarantees(&self) -> TelemetryGuarantees;
}

/// Emits one metric data point. Deliberately separate from [`TelemetrySink`]
/// and [`SpanSink`] for the same reason: a metric is a numeric, aggregatable
/// signal, not a discrete log or span.
pub trait MetricSink: Send + Sync {
    fn emit(&self, envelope: MetricEnvelopeV1) -> Result<(), ObservabilityError>;
    fn guarantees(&self) -> TelemetryGuarantees;
}

impl<T: TelemetrySink + ?Sized> TelemetrySink for Arc<T> {
    fn emit(&self, envelope: TelemetryEnvelopeV1) -> Result<(), ObservabilityError> {
        (**self).emit(envelope)
    }

    fn guarantees(&self) -> TelemetryGuarantees {
        (**self).guarantees()
    }
}
impl<T: TelemetryReader + ?Sized> TelemetryReader for Arc<T> {
    fn query(
        &self,
        tenant_id: &TenantId,
        query: &TelemetryQueryV1,
    ) -> Result<Vec<TelemetryRecordV1>, ObservabilityError> {
        (**self).query(tenant_id, query)
    }

    fn guarantees(&self) -> TelemetryGuarantees {
        (**self).guarantees()
    }
}
impl<T: Clock + ?Sized> Clock for Arc<T> {
    fn now(&self) -> Result<Timestamp, ObservabilityError> {
        (**self).now()
    }
}
impl<T: SpanSink + ?Sized> SpanSink for Arc<T> {
    fn emit(&self, envelope: SpanEnvelopeV1) -> Result<(), ObservabilityError> {
        (**self).emit(envelope)
    }

    fn guarantees(&self) -> TelemetryGuarantees {
        (**self).guarantees()
    }
}
impl<T: MetricSink + ?Sized> MetricSink for Arc<T> {
    fn emit(&self, envelope: MetricEnvelopeV1) -> Result<(), ObservabilityError> {
        (**self).emit(envelope)
    }

    fn guarantees(&self) -> TelemetryGuarantees {
        (**self).guarantees()
    }
}
