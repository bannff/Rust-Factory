#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]

//! Transport-independent trusted context and capability grant policy contracts.

use std::collections::BTreeSet;
use std::fmt;

use sha2::{Digest, Sha256};

pub const MAX_LOGICAL_ID_BYTES: usize = 128;
pub const MAX_TOOL_IDS: usize = 64;
pub const MAX_TOOL_ID_BYTES: usize = 128;

macro_rules! logical_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, PolicyError> {
                let value = value.into();
                is_logical_id(&value)
                    .then_some(Self(value))
                    .ok_or(PolicyError::InvalidId)
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
    };
}

logical_id!(TenantId);
logical_id!(PrincipalId);
logical_id!(RequestId);
logical_id!(CorrelationId);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TrustedContextV1 {
    pub tenant_id: TenantId,
    pub principal_id: PrincipalId,
    pub request_id: RequestId,
    pub correlation_id: CorrelationId,
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CapabilityV1 {
    AgentDefinitionValidate,
    AgentDefinitionGet,
    AgentDefinitionList,
    AgentDefinitionRegister,
    AgentInvoke,
    WorkflowValidate,
    WorkflowStart,
    WorkflowGet,
    WorkflowList,
    WorkflowCancel,
    EvaluationValidate,
    EvaluationEvaluate,
    EvaluationGet,
}
impl CapabilityV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::AgentDefinitionValidate => "agent_definition_validate",
            Self::AgentDefinitionGet => "agent_definition_get",
            Self::AgentDefinitionList => "agent_definition_list",
            Self::AgentDefinitionRegister => "agent_definition_register",
            Self::AgentInvoke => "agent_invoke",
            Self::WorkflowValidate => "workflow_validate",
            Self::WorkflowStart => "workflow_start",
            Self::WorkflowGet => "workflow_get",
            Self::WorkflowList => "workflow_list",
            Self::WorkflowCancel => "workflow_cancel",
            Self::EvaluationValidate => "evaluation_validate",
            Self::EvaluationEvaluate => "evaluation_evaluate",
            Self::EvaluationGet => "evaluation_get",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct GrantV1 {
    pub allowed_tool_ids: Vec<String>,
    pub memory_enabled: bool,
    pub knowledge_enabled: bool,
    pub sandbox_execution_allowed: bool,
    pub communication_allowed: bool,
}
impl GrantV1 {
    #[allow(clippy::fn_params_excessive_bools)] // The wire contract has four independent capability booleans.
    pub fn new(
        allowed_tool_ids: impl IntoIterator<Item = String>,
        memory_enabled: bool,
        knowledge_enabled: bool,
        sandbox_execution_allowed: bool,
        communication_allowed: bool,
    ) -> Result<Self, PolicyError> {
        let grant = Self {
            allowed_tool_ids: canonical_tools(allowed_tool_ids)?,
            memory_enabled,
            knowledge_enabled,
            sandbox_execution_allowed,
            communication_allowed,
        };
        validate_grant(&grant)?;
        Ok(grant)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationRequestV1 {
    pub context: TrustedContextV1,
    pub capability: CapabilityV1,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SafeDenyReasonV1 {
    Denied,
}
impl SafeDenyReasonV1 {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        "denied"
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthorizationDecisionV1 {
    Allow {
        effective_grant: GrantV1,
        decision_digest: String,
    },
    Deny {
        safe_reason: SafeDenyReasonV1,
    },
}

pub trait PolicyResolver: Send + Sync {
    fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicErrorCode {
    InvalidId,
    InvalidGrant,
    LimitExceeded,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PolicyError {
    InvalidId,
    InvalidGrant,
    LimitExceeded,
}
impl PolicyError {
    #[must_use]
    pub const fn public_code(self) -> PublicErrorCode {
        match self {
            Self::InvalidId => PublicErrorCode::InvalidId,
            Self::InvalidGrant => PublicErrorCode::InvalidGrant,
            Self::LimitExceeded => PublicErrorCode::LimitExceeded,
        }
    }
}
impl fmt::Display for PolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "policy operation failed: {:?}",
            self.public_code()
        )
    }
}
impl std::error::Error for PolicyError {}

pub fn validate_grant(grant: &GrantV1) -> Result<(), PolicyError> {
    if grant.allowed_tool_ids.len() > MAX_TOOL_IDS {
        return Err(PolicyError::LimitExceeded);
    }
    if grant
        .allowed_tool_ids
        .iter()
        .any(|tool_id| !is_tool_id(tool_id))
    {
        return Err(PolicyError::InvalidGrant);
    }
    Ok(())
}

pub fn canonical_grant(grant: &GrantV1) -> Result<GrantV1, PolicyError> {
    GrantV1::new(
        grant.allowed_tool_ids.clone(),
        grant.memory_enabled,
        grant.knowledge_enabled,
        grant.sandbox_execution_allowed,
        grant.communication_allowed,
    )
}

pub fn grant_canonical_bytes(grant: &GrantV1) -> Result<Vec<u8>, PolicyError> {
    let grant = canonical_grant(grant)?;
    let mut bytes = Vec::new();
    fields(
        &mut bytes,
        &["policy-grant-v1", &grant.allowed_tool_ids.len().to_string()],
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

pub fn grant_digest(grant: &GrantV1) -> Result<String, PolicyError> {
    Ok(digest(&grant_canonical_bytes(grant)?))
}

pub fn decision_canonical_bytes(
    request: &AuthorizationRequestV1,
    decision: &AuthorizationDecisionV1,
) -> Result<Vec<u8>, PolicyError> {
    let context = &request.context;
    let mut bytes = Vec::new();
    fields(
        &mut bytes,
        &[
            "policy-decision-v1",
            context.tenant_id.as_str(),
            context.principal_id.as_str(),
            context.request_id.as_str(),
            context.correlation_id.as_str(),
            request.capability.as_str(),
        ],
    );
    match decision {
        AuthorizationDecisionV1::Allow {
            effective_grant, ..
        } => {
            let grant = canonical_grant(effective_grant)?;
            fields(&mut bytes, &["allow", "grant-present"]);
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
            fields(&mut bytes, &["deny", safe_reason.as_str(), "grant-absent"]);
        }
    }
    Ok(bytes)
}

pub fn decision_digest(
    request: &AuthorizationRequestV1,
    decision: &AuthorizationDecisionV1,
) -> Result<String, PolicyError> {
    Ok(digest(&decision_canonical_bytes(request, decision)?))
}

pub fn allow_decision(
    request: &AuthorizationRequestV1,
    effective_grant: &GrantV1,
) -> Result<AuthorizationDecisionV1, PolicyError> {
    let effective_grant = canonical_grant(effective_grant)?;
    let provisional = AuthorizationDecisionV1::Allow {
        effective_grant: effective_grant.clone(),
        decision_digest: String::new(),
    };
    Ok(AuthorizationDecisionV1::Allow {
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

fn canonical_tools(tools: impl IntoIterator<Item = String>) -> Result<Vec<String>, PolicyError> {
    let tools: BTreeSet<_> = tools.into_iter().collect();
    if tools.len() > MAX_TOOL_IDS {
        return Err(PolicyError::LimitExceeded);
    }
    if tools.iter().any(|tool_id| !is_tool_id(tool_id)) {
        return Err(PolicyError::InvalidGrant);
    }
    Ok(tools.into_iter().collect())
}
fn is_logical_id(value: &str) -> bool {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn request() -> AuthorizationRequestV1 {
        AuthorizationRequestV1 {
            context: TrustedContextV1 {
                tenant_id: TenantId::new("tenant").expect("tenant"),
                principal_id: PrincipalId::new("principal").expect("principal"),
                request_id: RequestId::new("request").expect("request"),
                correlation_id: CorrelationId::new("correlation").expect("correlation"),
            },
            capability: CapabilityV1::AgentInvoke,
        }
    }
    fn grant() -> GrantV1 {
        GrantV1::new(
            ["tool-b".to_owned(), "tool-a".to_owned()],
            true,
            false,
            true,
            false,
        )
        .expect("grant")
    }

    #[test]
    fn ids_reject_empty_uppercase_and_overlong_values() {
        for value in ["", "Tenant", &"a".repeat(MAX_LOGICAL_ID_BYTES + 1)] {
            assert_eq!(TenantId::new(value), Err(PolicyError::InvalidId));
        }
    }

    #[test]
    fn every_logical_id_enforces_the_v1_grammar_and_byte_boundary() {
        let valid = "a".repeat(MAX_LOGICAL_ID_BYTES);
        assert!(TenantId::new(valid.clone()).is_ok());
        assert!(PrincipalId::new(valid.clone()).is_ok());
        assert!(RequestId::new(valid.clone()).is_ok());
        assert!(CorrelationId::new(valid).is_ok());

        for invalid in ["-leading", "with.dot", "with space", "café"] {
            assert_eq!(TenantId::new(invalid), Err(PolicyError::InvalidId));
            assert_eq!(PrincipalId::new(invalid), Err(PolicyError::InvalidId));
            assert_eq!(RequestId::new(invalid), Err(PolicyError::InvalidId));
            assert_eq!(CorrelationId::new(invalid), Err(PolicyError::InvalidId));
        }
    }

    #[test]
    fn grants_enforce_tool_grammar_and_distinct_tool_limit() {
        let maximum = (0..MAX_TOOL_IDS)
            .map(|index| format!("tool-{index}"))
            .collect::<Vec<_>>();
        assert_eq!(
            GrantV1::new(maximum, false, false, false, false)
                .expect("maximum distinct tools")
                .allowed_tool_ids
                .len(),
            MAX_TOOL_IDS
        );
        let over_limit = (0..=MAX_TOOL_IDS)
            .map(|index| format!("tool-{index}"))
            .collect::<Vec<_>>();
        assert_eq!(
            GrantV1::new(over_limit, false, false, false, false),
            Err(PolicyError::LimitExceeded)
        );
        for invalid in ["", "Tool", "tool/child", &"a".repeat(MAX_TOOL_ID_BYTES + 1)] {
            assert_eq!(
                GrantV1::new([invalid.to_owned()], false, false, false, false),
                Err(PolicyError::InvalidGrant)
            );
        }
    }

    #[test]
    fn grant_is_canonicalized_and_empty_tools_remain_explicit() {
        let canonical = GrantV1::new(
            [
                "tool-b".to_owned(),
                "tool-a".to_owned(),
                "tool-a".to_owned(),
            ],
            false,
            false,
            false,
            false,
        )
        .expect("grant");
        assert_eq!(canonical.allowed_tool_ids, ["tool-a", "tool-b"]);
        assert_eq!(
            canonical_grant(&GrantV1 {
                allowed_tool_ids: vec![],
                memory_enabled: false,
                knowledge_enabled: false,
                sandbox_execution_allowed: false,
                communication_allowed: false,
            })
            .expect("empty grant")
            .allowed_tool_ids,
            Vec::<String>::new()
        );
    }

    #[test]
    fn decision_and_grant_golden_vectors_are_stable() {
        let request = request();
        let grant = grant();
        let allow = allow_decision(&request, &grant).expect("allow");
        assert_eq!(
            String::from_utf8(grant_canonical_bytes(&grant).expect("bytes")).expect("utf8"),
            "15:policy-grant-v1\n1:2\n6:tool-a\n6:tool-b\n4:true\n5:false\n4:true\n5:false\n"
        );
        assert_eq!(
            String::from_utf8(decision_canonical_bytes(&request, &allow).expect("bytes"))
                .expect("utf8"),
            "18:policy-decision-v1\n6:tenant\n9:principal\n7:request\n11:correlation\n12:agent_invoke\n5:allow\n13:grant-present\n1:2\n6:tool-a\n6:tool-b\n4:true\n5:false\n4:true\n5:false\n"
        );
        assert_eq!(
            grant_digest(&grant).expect("digest"),
            "212af08f2a694402208bbe4b450dae27d4c563d46f4d89155ba5521a5af26a16"
        );
        assert_eq!(
            decision_digest(&request, &allow).expect("digest"),
            "71fd0de10592426a1b31ca9bb3de2661fa4517267bf8940aea5648ff3b88c3eb"
        );
    }

    #[test]
    fn deny_decision_uses_the_fixed_grant_absent_wire_format() {
        let request = request();
        let deny = deny_decision();
        assert_eq!(
            String::from_utf8(decision_canonical_bytes(&request, &deny).expect("bytes"))
                .expect("utf8"),
            "18:policy-decision-v1\n6:tenant\n9:principal\n7:request\n11:correlation\n12:agent_invoke\n4:deny\n6:denied\n12:grant-absent\n"
        );
    }

    #[test]
    fn decision_digest_binds_every_request_field_and_effective_grant() {
        let request = request();
        let decision = allow_decision(&request, &grant()).expect("allow");
        let original = decision_digest(&request, &decision).expect("digest");

        let mut other = request.clone();
        other.context.tenant_id = TenantId::new("other-tenant").expect("tenant");
        assert_ne!(
            original,
            decision_digest(&other, &decision).expect("digest")
        );
        other = request.clone();
        other.context.principal_id = PrincipalId::new("other-principal").expect("principal");
        assert_ne!(
            original,
            decision_digest(&other, &decision).expect("digest")
        );
        other = request.clone();
        other.context.request_id = RequestId::new("other-request").expect("request");
        assert_ne!(
            original,
            decision_digest(&other, &decision).expect("digest")
        );
        other = request.clone();
        other.context.correlation_id =
            CorrelationId::new("other-correlation").expect("correlation");
        assert_ne!(
            original,
            decision_digest(&other, &decision).expect("digest")
        );
        other = request.clone();
        other.capability = CapabilityV1::WorkflowStart;
        assert_ne!(
            original,
            decision_digest(&other, &decision).expect("digest")
        );

        let narrower =
            GrantV1::new(["tool-a".to_owned()], true, false, true, false).expect("grant");
        let changed_grant = allow_decision(&request, &narrower).expect("allow");
        assert_ne!(
            original,
            decision_digest(&request, &changed_grant).expect("digest")
        );
    }

    #[test]
    fn decision_digest_is_bound_to_context_and_capability() {
        let request = request();
        let grant = grant();
        let decision = allow_decision(&request, &grant).expect("allow");
        let mut other = request.clone();
        other.context.request_id = RequestId::new("other").expect("request");
        assert_ne!(
            decision_digest(&request, &decision).expect("digest"),
            decision_digest(&other, &decision).expect("digest")
        );
        other = request.clone();
        other.capability = CapabilityV1::WorkflowStart;
        assert_ne!(
            decision_digest(&request, &decision).expect("digest"),
            decision_digest(&other, &decision).expect("digest")
        );
    }
}
