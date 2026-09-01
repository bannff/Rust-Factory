use crate::{
    MAX_ATTRIBUTE_VALUE_BYTES, MAX_ATTRIBUTES, MAX_BAGGAGE_ENTRIES, MAX_BAGGAGE_VALUE_BYTES,
    MAX_BODY_BYTES, MAX_EVENT_BYTES, MAX_IDENTIFIER_BYTES, MAX_METRIC_BYTES, MAX_METRIC_UNIT_BYTES,
    MAX_SPAN_BYTES, MAX_TRACESTATE_BYTES, MetricEventV1, ObservabilityError, SpanEventV1,
    TelemetryEnvelopeV1, TelemetryEventV1, TelemetryQueryV1, TelemetryRecordV1, TraceContextV1,
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

pub fn validate_trace_context(context: &TraceContextV1) -> Result<(), ObservabilityError> {
    if let Some(state) = &context.trace_state {
        if state.len() > MAX_TRACESTATE_BYTES {
            return Err(ObservabilityError::LimitExceeded);
        }
        if state.bytes().any(is_control_byte) {
            return Err(ObservabilityError::InvalidTraceContext);
        }
    }
    if context.baggage.len() > MAX_BAGGAGE_ENTRIES {
        return Err(ObservabilityError::LimitExceeded);
    }
    for (key, value) in &context.baggage {
        validate_label(key).ok_or(ObservabilityError::InvalidTraceContext)?;
        if value.len() > MAX_BAGGAGE_VALUE_BYTES {
            return Err(ObservabilityError::LimitExceeded);
        }
        // Reject control bytes (covers CR/LF header-injection surface) and
        // the reserved baggage delimiters (`,`/`=`/`;`) that would otherwise
        // be reinterpreted as a new key/value boundary on re-parse by
        // `parse_baggage`, breaking format_baggage/parse_baggage round-trip
        // fidelity. This is a bounded rejection, not the W3C Baggage spec's
        // percent-encoding escape grammar, which this crate does not
        // implement.
        if value
            .bytes()
            .any(|byte| is_control_byte(byte) || matches!(byte, b',' | b'=' | b';'))
        {
            return Err(ObservabilityError::InvalidTraceContext);
        }
    }
    Ok(())
}

/// A C0 control byte (0x00-0x1F) or DEL (0x7F), covering CR/LF.
const fn is_control_byte(byte: u8) -> bool {
    byte <= 0x1F || byte == 0x7F
}

pub fn validate_span(span: &SpanEventV1) -> Result<(), ObservabilityError> {
    validate_event_name(span.name.as_str()).map_err(|_| ObservabilityError::InvalidSpan)?;
    validate_event_target(span.target.as_str()).map_err(|_| ObservabilityError::InvalidSpan)?;
    if span.end < span.start {
        return Err(ObservabilityError::InvalidSpan);
    }
    if span.attributes.len() > MAX_ATTRIBUTES {
        return Err(ObservabilityError::LimitExceeded);
    }
    for (key, value) in &span.attributes {
        validate_attribute_key(key).map_err(|_| ObservabilityError::InvalidSpan)?;
        if value.len() > MAX_ATTRIBUTE_VALUE_BYTES {
            return Err(ObservabilityError::LimitExceeded);
        }
    }
    let size = span
        .name
        .as_str()
        .len()
        .checked_add(span.target.as_str().len())
        .and_then(|size| {
            span.attributes.iter().try_fold(size, |size, (key, value)| {
                size.checked_add(key.len())
                    .and_then(|size| size.checked_add(value.len()))
            })
        })
        .ok_or(ObservabilityError::LimitExceeded)?;
    (size <= MAX_SPAN_BYTES)
        .then_some(())
        .ok_or(ObservabilityError::LimitExceeded)
}

pub fn validate_metric(metric: &MetricEventV1) -> Result<(), ObservabilityError> {
    validate_event_name(metric.name.as_str()).map_err(|_| ObservabilityError::InvalidMetric)?;
    if !metric.value.is_finite() {
        return Err(ObservabilityError::InvalidMetric);
    }
    if metric
        .unit
        .as_ref()
        .is_some_and(|unit| unit.len() > MAX_METRIC_UNIT_BYTES)
    {
        return Err(ObservabilityError::LimitExceeded);
    }
    if metric.attributes.len() > MAX_ATTRIBUTES {
        return Err(ObservabilityError::LimitExceeded);
    }
    for (key, value) in &metric.attributes {
        validate_attribute_key(key).map_err(|_| ObservabilityError::InvalidMetric)?;
        if value.len() > MAX_ATTRIBUTE_VALUE_BYTES {
            return Err(ObservabilityError::LimitExceeded);
        }
    }
    let size = metric
        .name
        .as_str()
        .len()
        .checked_add(metric.unit.as_deref().map_or(0, str::len))
        .and_then(|size| {
            metric
                .attributes
                .iter()
                .try_fold(size, |size, (key, value)| {
                    size.checked_add(key.len())
                        .and_then(|size| size.checked_add(value.len()))
                })
        })
        .ok_or(ObservabilityError::LimitExceeded)?;
    (size <= MAX_METRIC_BYTES)
        .then_some(())
        .ok_or(ObservabilityError::LimitExceeded)
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
