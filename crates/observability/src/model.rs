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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct TelemetryGuarantees {
    pub durable_across_restart: bool,
    pub visible_across_processes: bool,
    pub delivery_confirmed: bool,
    pub queryable: bool,
}
