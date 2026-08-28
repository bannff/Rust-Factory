use crate::{
    MAX_ATTRIBUTE_VALUE_BYTES, MAX_ATTRIBUTES, MAX_BODY_BYTES, MAX_EVENT_BYTES,
    MAX_IDENTIFIER_BYTES, ObservabilityError, TelemetryEnvelopeV1, TelemetryEventV1,
    TelemetryQueryV1, TelemetryRecordV1,
};

pub fn validate_tenant_id(value: &str) -> Result<(), ObservabilityError> {
    let mut bytes = value.bytes();
    (value.len() <= MAX_IDENTIFIER_BYTES
        && matches!(bytes.next(), Some(byte) if byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        }))
    .then_some(())
    .ok_or(ObservabilityError::InvalidId)
}

pub fn validate_event_name(value: &str) -> Result<(), ObservabilityError> {
    validate_label(value).ok_or(ObservabilityError::InvalidEvent)
}

pub fn validate_event_target(value: &str) -> Result<(), ObservabilityError> {
    validate_label(value).ok_or(ObservabilityError::InvalidEvent)
}

pub fn validate_attribute_key(value: &str) -> Result<(), ObservabilityError> {
    validate_label(value).ok_or(ObservabilityError::InvalidEvent)
}

pub fn validate_event(event: &TelemetryEventV1) -> Result<(), ObservabilityError> {
    validate_event_name(event.name.as_str())?;
    validate_event_target(event.target.as_str())?;
    if event.body.len() > MAX_BODY_BYTES || event.attributes.len() > MAX_ATTRIBUTES {
        return Err(ObservabilityError::LimitExceeded);
    }
    for (key, value) in &event.attributes {
        validate_attribute_key(key)?;
        if value.len() > MAX_ATTRIBUTE_VALUE_BYTES {
            return Err(ObservabilityError::LimitExceeded);
        }
    }
    (event_size(event)? <= MAX_EVENT_BYTES)
        .then_some(())
        .ok_or(ObservabilityError::LimitExceeded)
}

pub fn validate_envelope(envelope: &TelemetryEnvelopeV1) -> Result<(), ObservabilityError> {
    validate_tenant_id(envelope.tenant_id.as_str())?;
    validate_event(&envelope.event)
}

pub fn validate_record(record: &TelemetryRecordV1) -> Result<(), ObservabilityError> {
    (record.sequence > 0)
        .then_some(())
        .ok_or(ObservabilityError::InvalidEvent)?;
    validate_envelope(&record.envelope)
}

pub fn validate_query(
    query: &TelemetryQueryV1,
    maximum_limit: usize,
) -> Result<(), ObservabilityError> {
    if query.limit == 0 {
        return Err(ObservabilityError::InvalidQuery);
    }
    if query.limit > maximum_limit {
        return Err(ObservabilityError::LimitExceeded);
    }
    if matches!((query.since, query.until), (Some(since), Some(until)) if since >= until) {
        return Err(ObservabilityError::InvalidQuery);
    }
    if let Some(name) = &query.event_name {
        validate_event_name(name.as_str()).map_err(|_| ObservabilityError::InvalidQuery)?;
    }
    if let Some(target) = &query.target {
        validate_event_target(target.as_str()).map_err(|_| ObservabilityError::InvalidQuery)?;
    }
    Ok(())
}

#[must_use]
pub fn record_matches(record: &TelemetryRecordV1, query: &TelemetryQueryV1) -> bool {
    let envelope = &record.envelope;
    query.since.is_none_or(|since| envelope.timestamp >= since)
        && query.until.is_none_or(|until| envelope.timestamp < until)
        && query
            .minimum_severity
            .is_none_or(|severity| envelope.event.severity >= severity)
        && query
            .event_name
            .as_ref()
            .is_none_or(|name| envelope.event.name == *name)
        && query
            .target
            .as_ref()
            .is_none_or(|target| envelope.event.target == *target)
}

fn validate_label(value: &str) -> Option<()> {
    let mut bytes = value.bytes();
    (value.len() <= MAX_IDENTIFIER_BYTES
        && matches!(bytes.next(), Some(byte) if byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        }))
    .then_some(())
}

fn event_size(event: &TelemetryEventV1) -> Result<usize, ObservabilityError> {
    event.attributes.iter().try_fold(
        event
            .name
            .as_str()
            .len()
            .checked_add(event.target.as_str().len())
            .and_then(|size| size.checked_add(event.body.len()))
            .ok_or(ObservabilityError::LimitExceeded)?,
        |size, (key, value)| {
            size.checked_add(key.len())
                .and_then(|size| size.checked_add(value.len()))
                .ok_or(ObservabilityError::LimitExceeded)
        },
    )
}
