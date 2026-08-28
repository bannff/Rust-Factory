//! Semantic validation.
//!
//! Deserializing a boundary DTO proves its shape, never its validity. Every
//! rule that makes a value meaningful lives here, so an adapter and an MCP
//! caller are held to exactly the same standard.

use crate::error::MemoryError;
use crate::model::{
    MAX_CONTENT_BYTES, MAX_ID_BYTES, MAX_METADATA_ENTRIES, MAX_METADATA_ENTRY_BYTES,
    MAX_PARTITION_RECORDS, MAX_QUERY_LIMIT, MAX_TAGS, MAX_TENANT_NAMESPACES, MAX_TERM_BYTES,
    MemoryKind, MemoryQuery, MemoryRecord,
};

/// Identifier grammar: lowercase ASCII alphanumerics, `_`, `-`, `.`, starting
/// alphanumeric, at most [`MAX_ID_BYTES`].
#[must_use]
pub fn is_logical_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_ID_BYTES
        && bytes[0].is_ascii_alphanumeric()
        && !bytes[0].is_ascii_uppercase()
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-' | b'.')
        })
}

/// Tag grammar, identical to an identifier's so tags stay portable across
/// backends that index them differently.
#[must_use]
pub fn is_tag(value: &str) -> bool {
    is_logical_id(value)
}

/// Validates a record against every content, tag, and metadata bound.
///
/// # Errors
///
/// [`MemoryError::InvalidRecord`] for empty content or a malformed tag,
/// [`MemoryError::LimitExceeded`] for anything oversized.
pub fn validate_record(record: &MemoryRecord) -> Result<(), MemoryError> {
    if record.content.is_empty() {
        return Err(MemoryError::InvalidRecord);
    }
    if record.content.len() > MAX_CONTENT_BYTES {
        return Err(MemoryError::LimitExceeded);
    }
    if record.tags.len() > MAX_TAGS {
        return Err(MemoryError::LimitExceeded);
    }
    if !record.tags.iter().all(|tag| is_tag(tag)) {
        return Err(MemoryError::InvalidRecord);
    }
    // Duplicate tags would make an "every tag present" filter ambiguous.
    let mut sorted = record.tags.clone();
    sorted.sort();
    let unique = sorted.len();
    sorted.dedup();
    if sorted.len() != unique {
        return Err(MemoryError::InvalidRecord);
    }
    if record.metadata.len() > MAX_METADATA_ENTRIES {
        return Err(MemoryError::LimitExceeded);
    }
    for (key, value) in &record.metadata {
        if key.is_empty() {
            return Err(MemoryError::InvalidRecord);
        }
        if key.len() > MAX_METADATA_ENTRY_BYTES || value.len() > MAX_METADATA_ENTRY_BYTES {
            return Err(MemoryError::LimitExceeded);
        }
    }
    Ok(())
}

/// Validates a query's bounds and cross-field consistency.
///
/// # Errors
///
/// [`MemoryError::LimitExceeded`] for a zero or oversized limit or term,
/// [`MemoryError::InvalidQuery`] for a malformed tag, a duplicate kind, or a
/// time window that cannot contain anything.
pub fn validate_query(query: &MemoryQuery) -> Result<(), MemoryError> {
    if query.limit == 0 || query.limit > MAX_QUERY_LIMIT {
        return Err(MemoryError::LimitExceeded);
    }
    if query.tags.len() > MAX_TAGS {
        return Err(MemoryError::LimitExceeded);
    }
    if !query.tags.iter().all(|tag| is_tag(tag)) {
        return Err(MemoryError::InvalidQuery);
    }
    if let Some(term) = &query.term {
        if term.is_empty() {
            return Err(MemoryError::InvalidQuery);
        }
        if term.len() > MAX_TERM_BYTES {
            return Err(MemoryError::LimitExceeded);
        }
    }
    // Bound the length before cloning and sorting. `tags` is checked before its
    // scan for the same reason: doing the work first and rejecting afterwards
    // lets a caller pay us to sort a huge vector it always intended to be
    // refused. There are only as many kinds as variants, so anything longer than
    // that already contains a duplicate.
    if query.kinds.len() > MemoryKind::all().len() {
        return Err(MemoryError::InvalidQuery);
    }
    // A repeated kind is accepted by `contains` but signals a caller bug, and
    // silently tolerating it makes the filter's meaning unclear.
    let mut kinds = query.kinds.clone();
    let supplied = kinds.len();
    kinds.sort();
    kinds.dedup();
    if kinds.len() != supplied {
        return Err(MemoryError::InvalidQuery);
    }
    // `until` is exclusive, so an equal pair selects nothing. Reject rather
    // than silently return empty, which would look like missing data.
    if let (Some(since), Some(until)) = (query.since, query.until)
        && since >= until
    {
        return Err(MemoryError::InvalidQuery);
    }
    Ok(())
}

/// Reports whether accepting a write would exceed a capacity ceiling.
///
/// Lives here rather than in an adapter because capacity is contract clause 8 of
/// [`crate::port::MemoryStore`] and its constants are core. Every adapter calls
/// this, so a ceiling cannot be enforced in one backend and forgotten in the
/// next, and no adapter has to name another adapter to find the rule.
///
/// `namespace_is_new` and `key_is_new` are passed rather than derived because
/// only the adapter can know them, and it already has to look both up to perform
/// the write.
///
/// # Errors
///
/// [`MemoryError::LimitExceeded`] when the tenant already holds
/// [`MAX_TENANT_NAMESPACES`] namespaces and this write would add another, or the
/// partition already holds [`MAX_PARTITION_RECORDS`] records and this write would
/// add another.
pub fn check_capacity(
    tenant_namespaces: usize,
    namespace_is_new: bool,
    partition_records: usize,
    key_is_new: bool,
) -> Result<(), MemoryError> {
    if namespace_is_new && tenant_namespaces >= MAX_TENANT_NAMESPACES {
        return Err(MemoryError::LimitExceeded);
    }
    // Replacing an existing key consumes no capacity, so a full partition stays
    // updatable. Refusing a replace would leave a full partition unrepairable.
    if key_is_new && partition_records >= MAX_PARTITION_RECORDS {
        return Err(MemoryError::LimitExceeded);
    }
    Ok(())
}
