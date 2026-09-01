//! W3C Trace Context extraction and injection.
//!
//! Implements the wire format from the W3C Trace Context specification:
//! `traceparent` (version-trace_id-parent_id-trace_flags) and an opaque
//! `tracestate` passthrough. This module performs no I/O and names no HTTP
//! or MCP transport type; callers extract a `traceparent`/`tracestate`
//! header pair (as plain strings) from whatever transport they use and pass
//! them here, and inject the formatted strings back into their own
//! transport's headers.
//!
//! Only W3C Trace Context version `00` is supported. An unsupported version
//! byte, or a value that fails hex decoding, structural field-count checks,
//! or the underlying `TraceId`/`SpanId`/`TraceContextV1` validation (e.g. an
//! all-zero trace or span id) is rejected.

use crate::{ObservabilityError, SpanId, TraceContextV1, TraceFlags, TraceId};

const SUPPORTED_VERSION: &str = "00";

/// Upper bound on one `key=value` baggage member's raw byte length
/// (identifier key plus `=` plus value plus a comma separator), used only
/// to size the upfront raw-input length gate in [`parse_baggage`]. Not a
/// per-member enforcement point itself: [`validate_trace_context`] remains
/// the authority for exact key/value validity and byte ceilings.
const MAX_BAGGAGE_MEMBER_BYTES: usize =
    crate::MAX_IDENTIFIER_BYTES + 1 + crate::MAX_BAGGAGE_VALUE_BYTES + 1;

/// Parses a `traceparent` header value into its trace id, parent span id,
/// and trace flags. Ignores any provided `tracestate`/`baggage`; combine
/// with [`parse_trace_state`]/[`parse_baggage`] via [`extract`] to build a
/// full [`TraceContextV1`].
pub fn parse_traceparent(value: &str) -> Result<(TraceId, SpanId, TraceFlags), ObservabilityError> {
    let mut fields = value.split('-');
    let version = fields
        .next()
        .ok_or(ObservabilityError::InvalidTraceContext)?;
    if version != SUPPORTED_VERSION {
        return Err(ObservabilityError::InvalidTraceContext);
    }
    let trace_id_hex = fields
        .next()
        .ok_or(ObservabilityError::InvalidTraceContext)?;
    let span_id_hex = fields
        .next()
        .ok_or(ObservabilityError::InvalidTraceContext)?;
    let flags_hex = fields
        .next()
        .ok_or(ObservabilityError::InvalidTraceContext)?;
    if fields.next().is_some() {
        return Err(ObservabilityError::InvalidTraceContext);
    }
    let trace_id = TraceId::new(decode_hex_array::<16>(trace_id_hex)?)?;
    let span_id = SpanId::new(decode_hex_array::<8>(span_id_hex)?)?;
    let flags = decode_hex_array::<1>(flags_hex)?[0];
    Ok((trace_id, span_id, TraceFlags::from_raw(flags)))
}

/// Formats a `traceparent` header value from a trace id, span id, and trace
/// flags. The produced string always uses version `00`.
#[must_use]
pub fn format_traceparent(trace_id: &TraceId, span_id: &SpanId, flags: TraceFlags) -> String {
    format!(
        "{SUPPORTED_VERSION}-{}-{}-{:02x}",
        encode_hex(trace_id.as_bytes()),
        encode_hex(span_id.as_bytes()),
        flags.as_raw()
    )
}

/// Validates a raw `tracestate` header value against the bounded-length
/// contract enforced by [`TraceContextV1::new`], returning it unchanged.
/// `tracestate` is intentionally treated as an opaque passthrough: its
/// internal vendor-specific structure is never parsed or rewritten here.
pub fn parse_trace_state(value: &str) -> Result<String, ObservabilityError> {
    (value.len() <= crate::MAX_TRACESTATE_BYTES)
        .then(|| value.to_owned())
        .ok_or(ObservabilityError::LimitExceeded)
}

/// Parses a W3C Baggage header value (`key1=value1,key2=value2`) into a
/// bounded map. Keys/values are trimmed of surrounding whitespace; any
/// baggage-spec metadata (`;property=...`) is discarded, not preserved.
pub fn parse_baggage(
    value: &str,
) -> Result<std::collections::BTreeMap<String, String>, ObservabilityError> {
    // Bound the raw input length upfront, mirroring `parse_trace_state`.
    // Without this, a value that repeats a single key (e.g. "k=v,k=v,...")
    // never grows the entry-count guard below past one distinct key, so an
    // attacker-chosen-length input would otherwise be scanned in full
    // before any size limit applied.
    if value.len() > crate::MAX_BAGGAGE_ENTRIES * (MAX_BAGGAGE_MEMBER_BYTES) {
        return Err(ObservabilityError::LimitExceeded);
    }
    let mut baggage = std::collections::BTreeMap::new();
    if value.trim().is_empty() {
        return Ok(baggage);
    }
    for member in value.split(',') {
        let member = member.split(';').next().unwrap_or("").trim();
        if member.is_empty() {
            continue;
        }
        let mut parts = member.splitn(2, '=');
        let key = parts
            .next()
            .ok_or(ObservabilityError::InvalidTraceContext)?
            .trim();
        let raw_value = parts
            .next()
            .ok_or(ObservabilityError::InvalidTraceContext)?
            .trim();
        if key.is_empty() {
            return Err(ObservabilityError::InvalidTraceContext);
        }
        if baggage.len() >= crate::MAX_BAGGAGE_ENTRIES {
            return Err(ObservabilityError::LimitExceeded);
        }
        baggage.insert(key.to_owned(), raw_value.to_owned());
    }
    Ok(baggage)
}

/// Formats a bounded baggage map back into a W3C Baggage header value.
/// Entries are emitted in key order for deterministic output.
#[must_use]
pub fn format_baggage(baggage: &std::collections::BTreeMap<String, String>) -> String {
    baggage
        .iter()
        .map(|(key, value)| format!("{key}={value}"))
        .collect::<Vec<_>>()
        .join(",")
}

/// Extracts a full [`TraceContextV1`] from a caller-supplied `traceparent`
/// (required), `tracestate` (optional), and Baggage header (optional). This
/// is the single entry point most transport adapters should call rather
/// than invoking the individual parse functions directly.
pub fn extract(
    traceparent: &str,
    trace_state: Option<&str>,
    baggage: Option<&str>,
) -> Result<TraceContextV1, ObservabilityError> {
    let (trace_id, span_id, flags) = parse_traceparent(traceparent)?;
    let trace_state = trace_state.map(parse_trace_state).transpose()?;
    let baggage = baggage.map(parse_baggage).transpose()?.unwrap_or_default();
    TraceContextV1::new(trace_id, span_id, flags, trace_state, baggage)
}

/// The formatted `(traceparent, tracestate, baggage)` header values for a
/// [`TraceContextV1`], ready for injection into an outbound request.
/// `tracestate`/`baggage` are `None` when the context carries no value for
/// that header, so a caller can skip setting an empty header.
#[must_use]
pub fn inject(context: &TraceContextV1) -> (String, Option<String>, Option<String>) {
    let traceparent = format_traceparent(&context.trace_id, &context.span_id, context.trace_flags);
    let trace_state = context.trace_state.clone();
    let baggage = (!context.baggage.is_empty()).then(|| format_baggage(&context.baggage));
    (traceparent, trace_state, baggage)
}

fn decode_hex_array<const N: usize>(value: &str) -> Result<[u8; N], ObservabilityError> {
    if value.len() != N * 2 {
        return Err(ObservabilityError::InvalidTraceContext);
    }
    let mut bytes = [0u8; N];
    for (index, byte) in bytes.iter_mut().enumerate() {
        let hi = hex_digit(value.as_bytes()[index * 2])?;
        let lo = hex_digit(value.as_bytes()[index * 2 + 1])?;
        *byte = (hi << 4) | lo;
    }
    Ok(bytes)
}

/// Lowercase hex digits only, per the W3C Trace Context spec's
/// `HEXDIGLC` grammar (implementations MUST reject non-lowercase hex in
/// `parent-id`/`trace-id`). Accepting uppercase would also make
/// [`format_traceparent`]'s always-lowercase output a lossy transform of a
/// parsed uppercase input, breaking `inject(extract(x)) == x`.
fn hex_digit(byte: u8) -> Result<u8, ObservabilityError> {
    match byte {
        b'0'..=b'9' => Ok(byte - b'0'),
        b'a'..=b'f' => Ok(byte - b'a' + 10),
        _ => Err(ObservabilityError::InvalidTraceContext),
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_trace_id() -> TraceId {
        TraceId::new([1u8; 16]).unwrap()
    }
    fn sample_span_id() -> SpanId {
        SpanId::new([2u8; 8]).unwrap()
    }

    #[test]
    fn round_trips_traceparent() {
        let trace_id = sample_trace_id();
        let span_id = sample_span_id();
        let flags = TraceFlags::from_raw(0x01);
        let header = format_traceparent(&trace_id, &span_id, flags);
        assert_eq!(
            header,
            "00-01010101010101010101010101010101-0202020202020202-01"
        );
        let (parsed_trace, parsed_span, parsed_flags) = parse_traceparent(&header).unwrap();
        assert_eq!(parsed_trace, trace_id);
        assert_eq!(parsed_span, span_id);
        assert!(parsed_flags.is_sampled());
    }

    #[test]
    fn rejects_unsupported_version_wrong_field_count_and_bad_hex() {
        assert!(
            parse_traceparent("01-01010101010101010101010101010101-0202020202020202-01").is_err()
        );
        assert!(parse_traceparent("00-0101-0202020202020202-01").is_err());
        assert!(
            parse_traceparent("00-01010101010101010101010101010101-0202020202020202-01-extra")
                .is_err()
        );
        assert!(
            parse_traceparent("00-zz010101010101010101010101010101-0202020202020202-01").is_err()
        );
    }

    #[test]
    fn rejects_all_zero_trace_and_span_ids() {
        assert!(
            parse_traceparent("00-00000000000000000000000000000000-0202020202020202-01").is_err()
        );
        assert!(
            parse_traceparent("00-01010101010101010101010101010101-0000000000000000-01").is_err()
        );
    }

    #[test]
    fn baggage_round_trips_and_ignores_metadata() {
        let mut expected = std::collections::BTreeMap::new();
        expected.insert("userId".to_owned(), "alice".to_owned());
        expected.insert("region".to_owned(), "us-east".to_owned());
        let parsed = parse_baggage("userId=alice;prop=1,region=us-east").unwrap();
        assert_eq!(parsed, expected);
        assert_eq!(format_baggage(&parsed), "region=us-east,userId=alice");
    }

    #[test]
    fn baggage_enforces_bounded_entry_count() {
        let many = (0..=crate::MAX_BAGGAGE_ENTRIES)
            .map(|index| format!("k{index}=v"))
            .collect::<Vec<_>>()
            .join(",");
        assert!(parse_baggage(&many).is_err());
    }

    // A raw input that repeats a single key never grows the distinct-key
    // count past 1, so the entry-count guard alone cannot bound it; the
    // upfront raw-length gate must reject an arbitrarily long such input.
    #[test]
    fn baggage_enforces_bounded_raw_input_length_even_with_a_single_repeated_key() {
        let repeated_single_key = "k=v,".repeat(100_000);
        assert_eq!(
            parse_baggage(&repeated_single_key),
            Err(ObservabilityError::LimitExceeded)
        );
    }

    #[test]
    fn empty_baggage_is_empty_map_not_error() {
        assert!(parse_baggage("").unwrap().is_empty());
        assert!(parse_baggage("   ").unwrap().is_empty());
    }

    #[test]
    fn extract_and_inject_round_trip() {
        let traceparent = "00-01010101010101010101010101010101-0202020202020202-01";
        let context = extract(traceparent, Some("vendor=state"), Some("k=v")).unwrap();
        let (injected_traceparent, injected_state, injected_baggage) = inject(&context);
        assert_eq!(injected_traceparent, traceparent);
        assert_eq!(injected_state, Some("vendor=state".to_owned()));
        assert_eq!(injected_baggage, Some("k=v".to_owned()));
    }

    #[test]
    fn extract_rejects_oversized_tracestate() {
        let traceparent = "00-01010101010101010101010101010101-0202020202020202-01";
        let oversized = "x".repeat(crate::MAX_TRACESTATE_BYTES + 1);
        assert!(extract(traceparent, Some(&oversized), None).is_err());
    }

    // Regression: `decode_hex_array`/`hex_digit` currently accept uppercase A-F
    // The W3C Trace Context spec's `parent-id`/`trace-id` grammar is
    // lowercase-hex-only; implementations MUST reject non-lowercase hex.
    // Uppercase acceptance would also make `format_traceparent`'s
    // always-lowercase output a lossy transform of the parsed input.
    #[test]
    fn parse_traceparent_rejects_uppercase_hex() {
        let trace_id_hex = format!("{}AB", "01".repeat(15));
        let span_id_hex = "0202020202AB0202";
        let header = format!("00-{trace_id_hex}-{span_id_hex}-AB");
        assert!(parse_traceparent(&header).is_err());
    }

    // trace_state must reject control bytes (covers CR/LF), closing the
    // latent header-injection surface for any future transport adapter that
    // writes this value verbatim into a raw header.
    #[test]
    fn trace_state_rejects_crlf_and_control_bytes() {
        let malicious = "vendor=1\r\nX-Injected: evil";
        assert!(
            parse_trace_state(malicious).is_ok(),
            "byte-length check alone does not reject this"
        );
        assert!(
            extract(
                "00-01010101010101010101010101010101-0202020202020202-01",
                Some(malicious),
                None,
            )
            .is_err(),
            "TraceContextV1::new's validate_trace_context must reject control bytes"
        );
    }

    // Baggage values containing reserved delimiters (`,`/`=`/`;`) must be
    // rejected: format_baggage -> parse_baggage cannot round-trip them
    // losslessly (the delimiter is reinterpreted as a new key/value
    // boundary on re-parse), which is a correctness bug independent of any
    // transport/injection concern.
    #[test]
    fn baggage_value_with_reserved_delimiters_is_rejected() {
        let mut baggage = std::collections::BTreeMap::new();
        baggage.insert("k".to_owned(), "v1,v2=x".to_owned());
        assert_eq!(
            TraceContextV1::new(
                sample_trace_id(),
                sample_span_id(),
                TraceFlags::from_raw(0),
                None,
                baggage,
            ),
            Err(ObservabilityError::InvalidTraceContext)
        );
    }

    // Regression: `parse_baggage` silently keeps the last occurrence when a
    // key repeats, rather than erroring or preserving both. This is
    // permitted by the W3C Baggage spec ("Deduplicating the list. Duplicate
    // keys MAY be removed.") but was previously untested.
    #[test]
    fn parse_baggage_last_occurrence_wins_on_duplicate_keys() {
        let parsed = parse_baggage("k=first,k=second").unwrap();
        assert_eq!(parsed.get("k").map(String::as_str), Some("second"));
        assert_eq!(parsed.len(), 1);
    }
}
