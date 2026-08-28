use std::sync::Arc;

use crate::{
    ObservabilityError, TelemetryEnvelopeV1, TelemetryGuarantees, TelemetryQueryV1,
    TelemetryRecordV1, TenantId, Timestamp,
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
