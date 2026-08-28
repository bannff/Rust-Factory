#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]

//! Bounded, tenant-scoped operational log telemetry.

mod error;
mod model;
mod port;
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
    EventName, EventTarget, MAX_ATTRIBUTE_VALUE_BYTES, MAX_ATTRIBUTES, MAX_BODY_BYTES,
    MAX_EVENT_BYTES, MAX_IDENTIFIER_BYTES, MAX_LOCAL_EVENTS_PER_TENANT, MAX_LOCAL_EVENTS_TOTAL,
    MAX_QUERY_LIMIT, Severity, TelemetryContext, TelemetryEnvelopeV1, TelemetryEventV1,
    TelemetryGuarantees, TelemetryQueryV1, TelemetryRecordV1, TenantId, Timestamp,
};
pub use port::{Clock, TelemetryReader, TelemetrySink};
pub use service::TelemetryService;
pub use validation::{
    record_matches, validate_envelope, validate_event, validate_query, validate_record,
};
