#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]

//! Deterministic process-local policy grant resolution.

use std::collections::BTreeMap;
use std::fmt;

use policy::{
    AuthorizationDecisionV1, AuthorizationRequestV1, CapabilityV1, GrantV1, PolicyError,
    PolicyResolver, PrincipalId, TenantId, allow_decision, canonical_grant, deny_decision,
};

pub const MAX_STATIC_GRANT_RECORDS: usize = 1024;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct GrantKeyV1 {
    pub tenant_id: TenantId,
    pub principal_id: PrincipalId,
    pub capability: CapabilityV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StaticPolicyResolverError {
    DuplicateGrantKey,
    InvalidGrant(PolicyError),
    LimitExceeded,
}
impl fmt::Display for StaticPolicyResolverError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DuplicateGrantKey => formatter
                .write_str("static policy resolver construction failed: duplicate grant key"),
            Self::InvalidGrant(error) => {
                write!(
                    formatter,
                    "static policy resolver construction failed: {error}"
                )
            }
            Self::LimitExceeded => formatter.write_str(
                "static policy resolver construction failed: static grant record limit exceeded",
            ),
        }
    }
}
impl std::error::Error for StaticPolicyResolverError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidGrant(error) => Some(error),
            Self::DuplicateGrantKey | Self::LimitExceeded => None,
        }
    }
}

#[derive(Clone, Default)]
pub struct StaticPolicyResolver {
    grants: BTreeMap<GrantKeyV1, GrantV1>,
}
impl StaticPolicyResolver {
    pub fn new(
        grants: impl IntoIterator<Item = (GrantKeyV1, GrantV1)>,
    ) -> Result<Self, StaticPolicyResolverError> {
        let mut resolved_grants = BTreeMap::new();
        for (key, grant) in grants {
            if resolved_grants.len() >= MAX_STATIC_GRANT_RECORDS {
                return Err(StaticPolicyResolverError::LimitExceeded);
            }
            let grant = canonical_grant(&grant).map_err(StaticPolicyResolverError::InvalidGrant)?;
            if resolved_grants.insert(key, grant).is_some() {
                return Err(StaticPolicyResolverError::DuplicateGrantKey);
            }
        }
        Ok(Self {
            grants: resolved_grants,
        })
    }
}
impl PolicyResolver for StaticPolicyResolver {
    fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
        let key = GrantKeyV1 {
            tenant_id: request.context.tenant_id.clone(),
            principal_id: request.context.principal_id.clone(),
            capability: request.capability,
        };
        self.grants
            .get(&key)
            .and_then(|grant| allow_decision(&request, grant).ok())
            .unwrap_or_else(deny_decision)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use policy::{CorrelationId, RequestId, SafeDenyReasonV1, TrustedContextV1};

    fn tenant(value: &str) -> TenantId {
        TenantId::new(value).expect("tenant")
    }
    fn principal(value: &str) -> PrincipalId {
        PrincipalId::new(value).expect("principal")
    }
    fn request(tenant_id: &str, principal_id: &str) -> AuthorizationRequestV1 {
        AuthorizationRequestV1 {
            context: TrustedContextV1 {
                tenant_id: tenant(tenant_id),
                principal_id: principal(principal_id),
                request_id: RequestId::new("request").expect("request"),
                correlation_id: CorrelationId::new("correlation").expect("correlation"),
            },
            capability: CapabilityV1::AgentInvoke,
        }
    }
    fn grant() -> GrantV1 {
        GrantV1::new(["tool".to_owned()], true, false, false, false).expect("grant")
    }
    fn key(tenant_value: &str, principal_value: &str) -> GrantKeyV1 {
        GrantKeyV1 {
            tenant_id: tenant(tenant_value),
            principal_id: principal(principal_value),
            capability: CapabilityV1::AgentInvoke,
        }
    }

    #[test]
    fn unknown_context_and_capability_default_to_safe_deny() {
        let resolver =
            StaticPolicyResolver::new([(key("tenant", "principal"), grant())]).expect("resolver");
        assert_eq!(
            resolver.authorize(request("other", "principal")),
            AuthorizationDecisionV1::Deny {
                safe_reason: SafeDenyReasonV1::Denied
            }
        );
        let mut other_capability = request("tenant", "principal");
        other_capability.capability = CapabilityV1::WorkflowStart;
        assert!(matches!(
            resolver.authorize(other_capability),
            AuthorizationDecisionV1::Deny { .. }
        ));
    }

    #[test]
    fn tenant_and_principal_are_both_required_for_a_grant() {
        let resolver = StaticPolicyResolver::new([(key("tenant-a", "principal-a"), grant())])
            .expect("resolver");
        assert!(matches!(
            resolver.authorize(request("tenant-a", "principal-a")),
            AuthorizationDecisionV1::Allow { .. }
        ));
        assert!(matches!(
            resolver.authorize(request("tenant-b", "principal-a")),
            AuthorizationDecisionV1::Deny { .. }
        ));
        assert!(matches!(
            resolver.authorize(request("tenant-a", "principal-b")),
            AuthorizationDecisionV1::Deny { .. }
        ));
    }

    #[test]
    fn duplicate_static_grant_keys_are_rejected() {
        assert!(matches!(
            StaticPolicyResolver::new([
                (key("tenant", "principal"), grant()),
                (key("tenant", "principal"), grant()),
            ]),
            Err(StaticPolicyResolverError::DuplicateGrantKey)
        ));
    }

    #[test]
    fn malformed_static_grants_are_rejected() {
        assert!(matches!(
            StaticPolicyResolver::new([(
                key("tenant", "principal"),
                GrantV1 {
                    allowed_tool_ids: vec!["invalid/tool".to_owned()],
                    memory_enabled: true,
                    knowledge_enabled: true,
                    sandbox_execution_allowed: true,
                    communication_allowed: true,
                },
            )]),
            Err(StaticPolicyResolverError::InvalidGrant(
                PolicyError::InvalidGrant
            ))
        ));
    }

    #[test]
    fn static_grant_record_limit_is_enforced() {
        let grants = (0..=MAX_STATIC_GRANT_RECORDS)
            .map(|index| (key(&format!("tenant-{index}"), "principal"), grant()));
        assert!(matches!(
            StaticPolicyResolver::new(grants),
            Err(StaticPolicyResolverError::LimitExceeded)
        ));
    }

    #[test]
    fn resolver_emits_a_canonical_effective_grant_and_request_bound_digest() {
        let resolver = StaticPolicyResolver::new([(
            key("tenant", "principal"),
            GrantV1 {
                allowed_tool_ids: vec![
                    "tool-b".to_owned(),
                    "tool-a".to_owned(),
                    "tool-a".to_owned(),
                ],
                memory_enabled: true,
                knowledge_enabled: false,
                sandbox_execution_allowed: false,
                communication_allowed: false,
            },
        )])
        .expect("resolver");
        let first = resolver.authorize(request("tenant", "principal"));
        let second = resolver.authorize(AuthorizationRequestV1 {
            context: TrustedContextV1 {
                tenant_id: tenant("tenant"),
                principal_id: principal("principal"),
                request_id: RequestId::new("other-request").expect("request"),
                correlation_id: CorrelationId::new("correlation").expect("correlation"),
            },
            capability: CapabilityV1::AgentInvoke,
        });
        let AuthorizationDecisionV1::Allow {
            effective_grant,
            decision_digest,
        } = first
        else {
            panic!("expected allow");
        };
        assert_eq!(effective_grant.allowed_tool_ids, ["tool-a", "tool-b"]);
        assert!(matches!(second, AuthorizationDecisionV1::Allow { .. }));
        let AuthorizationDecisionV1::Allow {
            decision_digest: other_digest,
            ..
        } = second
        else {
            panic!("expected allow");
        };
        assert_ne!(decision_digest, other_digest);
    }
}
