use crate::KnowledgeError;

/// Maximum encoded length of a Knowledge identifier.
pub const MAX_IDENTIFIER_BYTES: usize = 128;
/// Maximum encoded length of a search query.
pub const MAX_QUERY_BYTES: usize = 16 * 1024;
/// Maximum number of hits requested or returned by one search.
pub const MAX_SEARCH_LIMIT: u32 = 64;
/// Maximum encoded text length of one knowledge document.
pub const MAX_DOCUMENT_TEXT_BYTES: usize = 16 * 1024;
/// Maximum aggregate encoded text length of one search result.
pub const MAX_RESULT_TEXT_BYTES: usize = 64 * 1024;
/// Maximum number of documents in a static corpus.
pub const MAX_STATIC_DOCUMENTS: usize = 10_000;
/// Maximum aggregate encoded text length of a static corpus.
pub const MAX_STATIC_TEXT_BYTES: usize = 64 * 1024 * 1024;

pub(crate) fn validate_identifier(value: &str) -> Result<(), KnowledgeError> {
    let bytes = value.as_bytes();
    if bytes.is_empty()
        || bytes.len() > MAX_IDENTIFIER_BYTES
        || !is_identifier_start(bytes[0])
        || !bytes[1..].iter().copied().all(is_identifier_continue)
    {
        return Err(KnowledgeError::InvalidRequest);
    }

    Ok(())
}

pub(crate) fn checked_add_with_limit(
    total: usize,
    additional: usize,
    limit: usize,
) -> Result<usize, KnowledgeError> {
    let total = total
        .checked_add(additional)
        .ok_or(KnowledgeError::LimitExceeded)?;
    if total > limit {
        return Err(KnowledgeError::LimitExceeded);
    }

    Ok(total)
}

const fn is_identifier_start(byte: u8) -> bool {
    byte.is_ascii_lowercase() || byte.is_ascii_digit()
}

const fn is_identifier_continue(byte: u8) -> bool {
    is_identifier_start(byte) || byte == b'_' || byte == b'-'
}

#[cfg(test)]
mod tests {
    use super::checked_add_with_limit;
    use crate::KnowledgeError;

    #[test]
    fn checked_add_with_limit_accepts_exact_limit() {
        assert_eq!(checked_add_with_limit(7, 5, 12), Ok(12));
    }

    #[test]
    fn checked_add_with_limit_rejects_value_over_limit() {
        assert_eq!(
            checked_add_with_limit(7, 6, 12),
            Err(KnowledgeError::LimitExceeded)
        );
    }

    #[test]
    fn checked_add_with_limit_classifies_usize_overflow_as_limit_exceeded() {
        assert_eq!(
            checked_add_with_limit(usize::MAX, 1, usize::MAX),
            Err(KnowledgeError::LimitExceeded)
        );
    }
}
