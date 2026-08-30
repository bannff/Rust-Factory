use std::collections::BTreeSet;

use crate::error::AuthError;
use crate::model::{AuthContextV1, AuthorizationRequestV1, GrantV1};

pub const MAX_LOGICAL_ID_BYTES: usize = 128;
pub const MAX_TOOL_IDS: usize = 64;
pub const MAX_TOOL_ID_BYTES: usize = 128;
/// Upper bound on a presented token, enforced by [`crate::TokenPresentation::new`].
///
/// This is the core structural ceiling. The `biscuit` adapter applies its own,
/// possibly tighter, cryptographic pre-verification ceiling before any Datalog
/// runs; see the feature-gated Biscuit adapter.
pub const MAX_TOKEN_PRESENTATION_BYTES: usize = 1 << 16;

pub fn validate_grant(grant: &GrantV1) -> Result<(), AuthError> {
    if grant.allowed_tool_ids.len() > MAX_TOOL_IDS {
        return Err(AuthError::LimitExceeded);
    }
    if grant
        .allowed_tool_ids
        .iter()
        .any(|tool_id| !is_tool_id(tool_id))
    {
        return Err(AuthError::InvalidGrant);
    }
    Ok(())
}

pub fn canonical_grant(grant: &GrantV1) -> Result<GrantV1, AuthError> {
    GrantV1::new(
        grant.allowed_tool_ids.clone(),
        grant.memory_enabled,
        grant.knowledge_enabled,
        grant.sandbox_execution_allowed,
        grant.communication_allowed,
    )
}

pub(crate) fn validate_request_scope(
    request: &AuthorizationRequestV1,
    context: &AuthContextV1,
) -> Result<(), AuthError> {
    if context.request_id != request.request_id || context.correlation_id != request.correlation_id
    {
        return Err(AuthError::InvalidId);
    }
    Ok(())
}

pub(crate) fn canonical_tools(
    tools: impl IntoIterator<Item = String>,
) -> Result<Vec<String>, AuthError> {
    let tools: BTreeSet<_> = tools.into_iter().collect();
    if tools.len() > MAX_TOOL_IDS {
        return Err(AuthError::LimitExceeded);
    }
    if tools.iter().any(|tool_id| !is_tool_id(tool_id)) {
        return Err(AuthError::InvalidGrant);
    }
    Ok(tools.into_iter().collect())
}

pub(crate) fn is_logical_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= MAX_LOGICAL_ID_BYTES
        && matches!(bytes.next(), Some(byte) if byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}

fn is_tool_id(value: &str) -> bool {
    let bytes = value.as_bytes();
    !bytes.is_empty()
        && bytes.len() <= MAX_TOOL_ID_BYTES
        && bytes.first().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.last().is_some_and(u8::is_ascii_alphanumeric)
        && bytes.iter().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_' | b'.')
        })
}

pub(crate) fn is_lowercase_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
