use sha2::{Digest, Sha256};

use crate::error::AuthError;
use crate::model::{
    AuthContextV1, AuthorizationDecisionV1, AuthorizationRequestV1, GrantV1, SafeDenyReasonV1,
};
use crate::validation::{canonical_grant, is_lowercase_sha256, validate_request_scope};

pub fn grant_canonical_bytes(grant: &GrantV1) -> Result<Vec<u8>, AuthError> {
    let grant = canonical_grant(grant)?;
    let mut bytes = Vec::new();
    fields(
        &mut bytes,
        &["auth-grant-v1", &grant.allowed_tool_ids.len().to_string()],
    );
    for tool_id in &grant.allowed_tool_ids {
        fields(&mut bytes, &[tool_id]);
    }
    fields(
        &mut bytes,
        &[
            boolean(grant.memory_enabled),
            boolean(grant.knowledge_enabled),
            boolean(grant.sandbox_execution_allowed),
            boolean(grant.communication_allowed),
        ],
    );
    Ok(bytes)
}

pub fn grant_digest(grant: &GrantV1) -> Result<String, AuthError> {
    Ok(digest(&grant_canonical_bytes(grant)?))
}

pub fn decision_canonical_bytes(
    request: &AuthorizationRequestV1,
    decision: &AuthorizationDecisionV1,
) -> Result<Vec<u8>, AuthError> {
    let mut bytes = Vec::new();
    match decision {
        AuthorizationDecisionV1::Allow {
            context,
            effective_grant,
            ..
        } => {
            validate_request_scope(request, context)?;
            let grant = canonical_grant(effective_grant)?;
            fields(
                &mut bytes,
                &[
                    "auth-decision-v1",
                    context.tenant_id.as_str(),
                    context.principal_id.as_str(),
                    request.request_id.as_str(),
                    request.correlation_id.as_str(),
                    request.capability.as_str(),
                    "allow",
                    "grant-present",
                ],
            );
            fields(&mut bytes, &[&grant.allowed_tool_ids.len().to_string()]);
            for tool_id in &grant.allowed_tool_ids {
                fields(&mut bytes, &[tool_id]);
            }
            fields(
                &mut bytes,
                &[
                    boolean(grant.memory_enabled),
                    boolean(grant.knowledge_enabled),
                    boolean(grant.sandbox_execution_allowed),
                    boolean(grant.communication_allowed),
                ],
            );
        }
        AuthorizationDecisionV1::Deny { safe_reason } => {
            // A denial derives no identity from the token, so only the
            // request-scoped fields and the opaque reason are bound.
            fields(
                &mut bytes,
                &[
                    "auth-decision-v1",
                    request.request_id.as_str(),
                    request.correlation_id.as_str(),
                    request.capability.as_str(),
                    "deny",
                    safe_reason.as_str(),
                    "grant-absent",
                ],
            );
        }
    }
    Ok(bytes)
}

pub fn decision_digest(
    request: &AuthorizationRequestV1,
    decision: &AuthorizationDecisionV1,
) -> Result<String, AuthError> {
    Ok(digest(&decision_canonical_bytes(request, decision)?))
}

/// Verifies that a decision is canonical and bound to `request`.
///
/// An allow verifies only when its request scope matches, its grant is already
/// canonical and valid, its digest is exactly 64 lowercase hexadecimal
/// characters, and that digest equals a fresh recomputation. The closed opaque
/// deny verifies because it carries no authority-bearing context or grant.
///
/// This is a consistency check, not authentication. The digest is unkeyed, so
/// anyone able to construct a decision can recompute it after changing fields.
/// Consumers must accept decisions only from a trusted injected
/// [`crate::AuthorizationResolver`]; `verify_decision` does not establish
/// provenance for caller-supplied or deserialized values.
#[must_use]
pub fn verify_decision(
    request: &AuthorizationRequestV1,
    decision: &AuthorizationDecisionV1,
) -> bool {
    match decision {
        AuthorizationDecisionV1::Allow {
            context,
            effective_grant,
            decision_digest: supplied_digest,
        } => {
            if validate_request_scope(request, context).is_err()
                || !is_lowercase_sha256(supplied_digest)
            {
                return false;
            }
            let Ok(canonical_grant) = canonical_grant(effective_grant) else {
                return false;
            };
            if canonical_grant != *effective_grant {
                return false;
            }
            let provisional = AuthorizationDecisionV1::Allow {
                context: context.clone(),
                effective_grant: canonical_grant,
                decision_digest: String::new(),
            };
            decision_digest(request, &provisional)
                .is_ok_and(|expected_digest| supplied_digest == &expected_digest)
        }
        AuthorizationDecisionV1::Deny {
            safe_reason: SafeDenyReasonV1::Denied,
        } => true,
    }
}

pub fn allow_decision(
    request: &AuthorizationRequestV1,
    context: &AuthContextV1,
    effective_grant: &GrantV1,
) -> Result<AuthorizationDecisionV1, AuthError> {
    validate_request_scope(request, context)?;
    let effective_grant = canonical_grant(effective_grant)?;
    let provisional = AuthorizationDecisionV1::Allow {
        context: context.clone(),
        effective_grant: effective_grant.clone(),
        decision_digest: String::new(),
    };
    Ok(AuthorizationDecisionV1::Allow {
        context: context.clone(),
        effective_grant,
        decision_digest: decision_digest(request, &provisional)?,
    })
}

#[must_use]
pub const fn deny_decision() -> AuthorizationDecisionV1 {
    AuthorizationDecisionV1::Deny {
        safe_reason: SafeDenyReasonV1::Denied,
    }
}

fn fields(bytes: &mut Vec<u8>, values: &[&str]) {
    for value in values {
        bytes.extend_from_slice(value.len().to_string().as_bytes());
        bytes.push(b':');
        bytes.extend_from_slice(value.as_bytes());
        bytes.push(b'\n');
    }
}

const fn boolean(value: bool) -> &'static str {
    if value { "true" } else { "false" }
}

fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
