use std::fmt;

use crate::error::AuthError;
use crate::validation::{
    MAX_TOKEN_PRESENTATION_BYTES, canonical_tools, is_logical_id, validate_grant,
};

macro_rules! logical_id {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, AuthError> {
                let value = value.into();
                is_logical_id(&value)
                    .then_some(Self(value))
                    .ok_or(AuthError::InvalidId)
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

/// The verified authorization context produced by token verification.
///
/// `tenant_id` and `principal_id` are identity derived from the
/// cryptographically verified token. `request_id` and `correlation_id` are
/// untrusted caller/request scope copied from [`AuthorizationRequestV1`]; they
/// bind a decision to one request but do not establish identity.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthContextV1 {
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
    ObservabilityTelemetryQuery,
    ObservabilityTelemetryStatus,
    MemoryRemember,
    MemoryRecall,
    MemorySearch,
    MemoryForget,
    MemoryStatus,
}

impl CapabilityV1 {
    /// The stable wire name for this capability.
    ///
    /// These names match `policy`'s capability wire names one-for-one so
    /// consumers can map an auth decision onto a policy grant without
    /// translation. They are bound into [`crate::decision_digest`], so they are
    /// permanent wire contracts rather than internal identifiers.
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
            Self::ObservabilityTelemetryQuery => "observability_telemetry_query",
            Self::ObservabilityTelemetryStatus => "observability_telemetry_status",
            Self::MemoryRemember => "memory_remember",
            Self::MemoryRecall => "memory_recall",
            Self::MemorySearch => "memory_search",
            Self::MemoryForget => "memory_forget",
            Self::MemoryStatus => "memory_status",
        }
    }
}

/// The effective capability ceiling a token permits.
///
/// Identical in shape to `policy`'s `GrantV1` so an auth decision maps cleanly
/// onto a policy grant.
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
    ) -> Result<Self, AuthError> {
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

/// Request-scoped authorization input.
///
/// Identity is deliberately absent: it is derived from the verified token, not
/// asserted by the caller. Only the request scope and the requested capability
/// are supplied here.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AuthorizationRequestV1 {
    pub request_id: RequestId,
    pub correlation_id: CorrelationId,
    pub capability: CapabilityV1,
}

/// Untrusted caller-supplied bearer credential bytes, opaque to the core.
///
/// Possession or presentation does not establish identity or trust. The core
/// only bounds the bytes; an adapter must verify the token signature and
/// Datalog against a public root key injected by the trusted host. The raw
/// credential must never enter logs, errors, or authorization evidence.
#[derive(Clone, Eq, PartialEq)]
pub struct TokenPresentation(String);

impl fmt::Debug for TokenPresentation {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_tuple("TokenPresentation")
            .field(&"[REDACTED]")
            .finish()
    }
}

impl TokenPresentation {
    pub fn new(raw: impl Into<String>) -> Result<Self, AuthError> {
        let raw = raw.into();
        if raw.is_empty() || raw.len() > MAX_TOKEN_PRESENTATION_BYTES {
            return Err(AuthError::InvalidToken);
        }
        Ok(Self(raw))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
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

/// The result of an authorization attempt.
///
/// `Deny` carries only [`SafeDenyReasonV1`], which has a single variant on
/// purpose: distinguishing a crypto failure, a parse failure, an expiry, and a
/// policy denial would leak an attack signal. Every failure mode collapses to
/// the same opaque denial.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AuthorizationDecisionV1 {
    Allow {
        context: AuthContextV1,
        effective_grant: GrantV1,
        decision_digest: String,
    },
    Deny {
        safe_reason: SafeDenyReasonV1,
    },
}
