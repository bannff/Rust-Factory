//! Bounded MCP control-plane adapter for agent definitions and local invocation.
//!
//! Enabled by the `mcp` feature. This module owns transport DTOs, generated
//! schemas, the policy gate, and safe response projection. It owns no process
//! lifecycle: a composition binary under `projects/` binds the transport.

#![allow(unknown_lints)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::unused_async_trait_impl)]

use crate::{
    AgentDefinitionV1, AgentId, AgentRegistry, CommunicationPolicy, CorrelationId, DefinitionError,
    DefinitionStore, DefinitionVersion, EffectiveCapabilityCeilingV1, ExecutionLimits,
    InvocationContextV1, KnowledgePolicy, LocalAgentRuntime, MAX_INPUT_BYTES, MemoryPolicy,
    MemoryStore, ModelPolicy, PrincipalId, PublicErrorCode, ReferenceCatalog, RequestId, Sandbox,
    SandboxPolicy, TenantId, ToolRegistry, validate_definition,
};
use anyhow::{Context, Result};
use policy::{
    AuthorizationDecisionV1, AuthorizationRequestV1, CapabilityV1, GrantV1, PolicyResolver,
    TrustedContextV1, canonical_grant, decision_digest,
};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const MAX_MCP_SERIALIZED_RESULT_BYTES: usize = 65_536;
pub const AGENT_DEFINITION_TOOLS: [&str; 5] = [
    "agent_definition_validate",
    "agent_definition_get",
    "agent_definition_list",
    "agent_definition_register",
    "agent_runtime_invoke",
];

/// Host-owned boundary that derives trusted request context independently of MCP input.
pub trait TrustedContextSource: Send + Sync {
    fn resolve(&self) -> std::result::Result<TrustedContextV1, DefinitionError>;
}

/// Owned request-scoped invocation controls supplied by composition.
pub trait InvocationControlBundle: Send {
    fn control(&self) -> llm_gateway::InvocationControl<'_>;
}

/// Host-owned factory for one request-scoped key, cancellation signal, and deadline signal.
///
/// Implementations provide wake mechanics but Agent creates no timer, runtime, or thread.
pub trait InvocationControlSource: Send + Sync {
    fn create(&self) -> std::result::Result<Box<dyn InvocationControlBundle>, DefinitionError>;
}

#[derive(Clone, Copy)]
enum PolicyGateError {
    Denied,
    Failed,
}

struct AuthorizedAgentContext {
    trusted: TrustedContextV1,
    grant: GrantV1,
}

/// Joins host-derived trusted identity with a verified closed policy decision.
pub struct AgentPolicyContextResolver<T, P> {
    source: T,
    policy: P,
}
impl<T, P> AgentPolicyContextResolver<T, P>
where
    T: TrustedContextSource,
    P: PolicyResolver,
{
    #[must_use]
    pub fn new(source: T, policy: P) -> Self {
        Self { source, policy }
    }

    fn resolve_and_authorize(
        &self,
        capability: CapabilityV1,
    ) -> std::result::Result<AuthorizedAgentContext, PolicyGateError> {
        let trusted = self.source.resolve().map_err(|_| PolicyGateError::Failed)?;
        let request = AuthorizationRequestV1 {
            context: trusted,
            capability,
        };
        let AuthorizationDecisionV1::Allow {
            effective_grant,
            decision_digest: supplied_digest,
        } = self.policy.authorize(request.clone())
        else {
            return Err(PolicyGateError::Denied);
        };
        let grant = canonical_grant(&effective_grant).map_err(|_| PolicyGateError::Failed)?;
        let expected_digest = decision_digest(
            &request,
            &AuthorizationDecisionV1::Allow {
                effective_grant: grant.clone(),
                decision_digest: String::new(),
            },
        )
        .map_err(|_| PolicyGateError::Failed)?;
        if supplied_digest != expected_digest {
            return Err(PolicyGateError::Failed);
        }
        Ok(AuthorizedAgentContext {
            trusted: request.context,
            grant,
        })
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)]
pub struct AgentDefinitionInput {
    pub id: String,
    pub name: String,
    pub description: String,
    pub model_reference: String,
    pub instructions: String,
    #[serde(default)]
    pub skills: Vec<String>,
    #[serde(default)]
    pub steering: Vec<String>,
    #[serde(default)]
    pub allowed_tool_ids: Vec<String>,
    pub memory_enabled: bool,
    pub memory_max_items: u32,
    pub knowledge_enabled: bool,
    pub knowledge_namespace: String,
    pub knowledge_max_results: u32,
    pub sandbox_allow_execution: bool,
    pub communication_allow_messages: bool,
    pub max_tool_calls: u32,
    pub max_output_bytes: u32,
}
impl AgentDefinitionInput {
    fn into_core(self) -> std::result::Result<AgentDefinitionV1, DefinitionError> {
        let definition = AgentDefinitionV1 {
            version: DefinitionVersion::V1,
            id: AgentId::new(self.id)?,
            name: self.name,
            description: self.description,
            model: ModelPolicy {
                reference: self.model_reference,
            },
            instructions: self.instructions,
            skills: self.skills,
            steering: self.steering,
            allowed_tool_ids: self.allowed_tool_ids,
            memory: MemoryPolicy {
                enabled: self.memory_enabled,
                max_items: self.memory_max_items,
            },
            knowledge: KnowledgePolicy {
                enabled: self.knowledge_enabled,
                namespace: self.knowledge_namespace,
                max_results: self.knowledge_max_results,
            },
            sandbox: SandboxPolicy {
                allow_execution: self.sandbox_allow_execution,
            },
            communication: CommunicationPolicy {
                allow_messages: self.communication_allow_messages,
            },
            limits: ExecutionLimits {
                max_tool_calls: self.max_tool_calls,
                max_output_bytes: self.max_output_bytes,
            },
        };
        validate_definition(&definition)?;
        Ok(definition)
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIdInput {
    pub id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvokeInput {
    pub id: String,
    pub input: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ListInput {}

/// MCP server whose core ports are injected by the embedding application.
pub struct AgentDefinitionMcp<S, C, M, T, MM, K, SB, TS, P>
where
    S: DefinitionStore,
    C: ReferenceCatalog,
    M: llm_gateway::LlmProvider,
    T: ToolRegistry,
    MM: MemoryStore,
    K: knowledge::KnowledgeIndex,
    SB: Sandbox,
    TS: TrustedContextSource,
    P: PolicyResolver,
{
    registry: AgentRegistry<S, C>,
    model: M,
    tools: T,
    memory: MM,
    knowledge: K,
    sandbox: SB,
    resolver: AgentPolicyContextResolver<TS, P>,
    invocation_control: Box<dyn InvocationControlSource>,
    tool_router: ToolRouter<Self>,
}
impl<S, C, M, T, MM, K, SB, TS, P> AgentDefinitionMcp<S, C, M, T, MM, K, SB, TS, P>
where
    S: DefinitionStore + 'static,
    C: ReferenceCatalog + 'static,
    M: llm_gateway::LlmProvider + 'static,
    T: ToolRegistry + 'static,
    MM: MemoryStore + 'static,
    K: knowledge::KnowledgeIndex + 'static,
    SB: Sandbox + 'static,
    TS: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        builtins: Vec<AgentDefinitionV1>,
        store: S,
        catalog: C,
        model: M,
        tools: T,
        memory: MM,
        knowledge: K,
        sandbox: SB,
        resolver: AgentPolicyContextResolver<TS, P>,
        invocation_control: Box<dyn InvocationControlSource>,
    ) -> std::result::Result<Self, DefinitionError> {
        Ok(Self {
            registry: AgentRegistry::new(builtins, store, catalog)?,
            model,
            tools,
            memory,
            knowledge,
            sandbox,
            resolver,
            invocation_control,
            tool_router: Self::tool_router(),
        })
    }

    fn validate_json(&self, input: AgentDefinitionInput) -> Result<String> {
        let result = (|| {
            let definition = input.into_core().map_err(domain_error)?;
            self.resolver
                .resolve_and_authorize(CapabilityV1::AgentDefinitionValidate)
                .map_err(policy_error)?;
            self.registry.validate(&definition).map_err(domain_error)
        })();
        match result {
            Ok(()) => serialize(json!({"valid": true, "findings": []})),
            Err(error) => {
                let code = error.to_string();
                serialize(
                    json!({"valid": false, "error": if is_public_code(&code) { code } else { "operation_failed".to_owned() }}),
                )
            }
        }
    }

    fn get_json(&self, input: AgentIdInput) -> Result<String> {
        let id = AgentId::new(input.id).map_err(|_| DefinitionError::InvalidDefinition)?;
        self.resolver
            .resolve_and_authorize(CapabilityV1::AgentDefinitionGet)
            .map_err(policy_error)?;
        let definition = self.registry.get(&id).map_err(domain_error)?;
        definition_json(&definition)
    }

    fn list_json(&self) -> Result<String> {
        self.resolver
            .resolve_and_authorize(CapabilityV1::AgentDefinitionList)
            .map_err(policy_error)?;
        let definitions = self.registry.list().map_err(domain_error)?;
        serialize(
            json!({"agents": definitions.into_iter().map(|definition| json!({"id": definition.id.as_str(), "name": definition.name, "description": definition.description})).collect::<Vec<_>>() }),
        )
    }

    fn register_json(&self, input: AgentDefinitionInput) -> Result<String> {
        let definition = input.into_core().map_err(domain_error)?;
        self.resolver
            .resolve_and_authorize(CapabilityV1::AgentDefinitionRegister)
            .map_err(policy_error)?;
        self.registry
            .register(definition.clone())
            .map_err(domain_error)?;
        serialize(json!({"id": definition.id.as_str(), "registered": true}))
    }

    async fn invoke_json(&self, input: InvokeInput) -> Result<String> {
        let id = AgentId::new(input.id).map_err(|_| anyhow::anyhow!("invalid_definition"))?;
        if input.input.len() > MAX_INPUT_BYTES {
            return Err(anyhow::anyhow!("limit_exceeded"));
        }
        let authorized = self
            .resolver
            .resolve_and_authorize(CapabilityV1::AgentInvoke)
            .map_err(policy_error)?;
        let context = invocation_context(authorized.trusted).map_err(domain_error)?;
        let ceiling = EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: authorized.grant.allowed_tool_ids,
            memory_enabled: authorized.grant.memory_enabled,
            knowledge_enabled: authorized.grant.knowledge_enabled,
            sandbox_execution_allowed: authorized.grant.sandbox_execution_allowed,
            communication_allowed: authorized.grant.communication_allowed,
        };
        let controls = self.invocation_control.create().map_err(domain_error)?;
        let runtime = LocalAgentRuntime::new(
            &self.registry,
            &self.model,
            &self.tools,
            &self.memory,
            &self.knowledge,
            &self.sandbox,
        );
        let result = runtime
            .invoke_with_ceiling(context, &id, input.input, &ceiling, controls.control())
            .await
            .map_err(domain_error)?;
        invocation_json(result)
    }
}

#[tool_router(router = tool_router)]
impl<S, C, M, T, MM, K, SB, TS, P> AgentDefinitionMcp<S, C, M, T, MM, K, SB, TS, P>
where
    S: DefinitionStore + 'static,
    C: ReferenceCatalog + 'static,
    M: llm_gateway::LlmProvider + 'static,
    T: ToolRegistry + 'static,
    MM: MemoryStore + 'static,
    K: knowledge::KnowledgeIndex + 'static,
    SB: Sandbox + 'static,
    TS: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    #[tool(
        name = "agent_definition_validate",
        description = "Validate a version-one agent definition without registering it."
    )]
    async fn agent_definition_validate(
        &self,
        Parameters(input): Parameters<AgentDefinitionInput>,
    ) -> String {
        tool_response(self.validate_json(input))
    }
    #[tool(
        name = "agent_definition_get",
        description = "Get one validated agent definition by logical ID."
    )]
    async fn agent_definition_get(&self, Parameters(input): Parameters<AgentIdInput>) -> String {
        tool_response(self.get_json(input))
    }
    #[tool(
        name = "agent_definition_list",
        description = "List safe discovery summaries for registered agents."
    )]
    async fn agent_definition_list(&self, Parameters(_): Parameters<ListInput>) -> String {
        tool_response(self.list_json())
    }
    #[tool(
        name = "agent_definition_register",
        description = "Register a validated user agent definition."
    )]
    async fn agent_definition_register(
        &self,
        Parameters(input): Parameters<AgentDefinitionInput>,
    ) -> String {
        tool_response(self.register_json(input))
    }
    #[tool(
        name = "agent_runtime_invoke",
        description = "Invoke one bounded local agent attempt through injected ports."
    )]
    async fn agent_runtime_invoke(&self, Parameters(input): Parameters<InvokeInput>) -> String {
        tool_response(self.invoke_json(input).await)
    }
}

#[tool_handler(router = self.tool_router)]
impl<S, C, M, T, MM, K, SB, TS, P> ServerHandler
    for AgentDefinitionMcp<S, C, M, T, MM, K, SB, TS, P>
where
    S: DefinitionStore + 'static,
    C: ReferenceCatalog + 'static,
    M: llm_gateway::LlmProvider + 'static,
    T: ToolRegistry + 'static,
    MM: MemoryStore + 'static,
    K: knowledge::KnowledgeIndex + 'static,
    SB: Sandbox + 'static,
    TS: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
}

fn invocation_context(
    trusted: TrustedContextV1,
) -> std::result::Result<InvocationContextV1, DefinitionError> {
    Ok(InvocationContextV1::new(
        TenantId::new(trusted.tenant_id.as_str())?,
        PrincipalId::new(trusted.principal_id.as_str())?,
        RequestId::new(trusted.request_id.as_str())?,
        CorrelationId::new(trusted.correlation_id.as_str())?,
    ))
}

fn policy_error(error: PolicyGateError) -> anyhow::Error {
    anyhow::anyhow!(match error {
        PolicyGateError::Denied => "not_found",
        PolicyGateError::Failed => "operation_failed",
    })
}
fn domain_error(error: DefinitionError) -> anyhow::Error {
    anyhow::anyhow!(public_code(error.public_code()))
}
fn definition_json(definition: &AgentDefinitionV1) -> Result<String> {
    serialize(
        json!({"version": definition.version.as_str(), "id": definition.id.as_str(), "name": definition.name, "description": definition.description, "model_reference": definition.model.reference, "instructions": definition.instructions, "skills": definition.skills, "steering": definition.steering, "allowed_tool_ids": definition.allowed_tool_ids, "memory": {"enabled": definition.memory.enabled, "max_items": definition.memory.max_items}, "knowledge": {"enabled": definition.knowledge.enabled, "namespace": definition.knowledge.namespace, "max_results": definition.knowledge.max_results}, "sandbox": {"allow_execution": definition.sandbox.allow_execution}, "communication": {"allow_messages": definition.communication.allow_messages}, "limits": {"max_tool_calls": definition.limits.max_tool_calls, "max_output_bytes": definition.limits.max_output_bytes}}),
    )
}
fn invocation_json(result: crate::InvocationResult) -> Result<String> {
    serialize(
        json!({"capability_scope_digest": result.capability_scope_digest, "events": result.events.into_iter().map(|event| match event {
            crate::InvocationEvent::ModelInvoked => json!({"type":"model_invoked"}),
            crate::InvocationEvent::MemoryRecalled { values } => json!({"type":"memory_recalled", "values":values}),
            crate::InvocationEvent::MemoryWritten => json!({"type":"memory_written"}),
            crate::InvocationEvent::KnowledgeSearched { results } => json!({"type":"knowledge_searched", "results":results.into_iter().map(|result| json!({"document_id":result.document_id,"text":result.text})).collect::<Vec<_>>() }),
            crate::InvocationEvent::SandboxCompleted { output } => json!({"type":"sandbox_completed", "output":output}),
            crate::InvocationEvent::ToolCompleted { tool_id, output } => json!({"type":"tool_completed", "tool_id":tool_id, "output":output}),
        }).collect::<Vec<_>>(), "output": result.output, "model_evidence": {
            "provider_id": result.model_evidence.provider_id,
            "model_id": result.model_evidence.model_id,
            "provider_request_id": result.model_evidence.provider_request_id,
            "finish_reason": match result.model_evidence.finish_reason {
                crate::InvocationModelFinishReason::Stop => "stop",
                crate::InvocationModelFinishReason::Length => "length",
                crate::InvocationModelFinishReason::ToolCalls => "tool_calls",
                crate::InvocationModelFinishReason::ContentFilter => "content_filter",
                crate::InvocationModelFinishReason::Other => "other",
            },
            "token_usage": result.model_evidence.token_usage.map(|usage| json!({
                "input_tokens": usage.input_tokens,
                "output_tokens": usage.output_tokens,
                "total_tokens": usage.total_tokens,
            })),
            "idempotency": match result.model_evidence.idempotency {
                crate::InvocationModelIdempotency::Unsupported => "unsupported",
                crate::InvocationModelIdempotency::Accepted => "accepted",
            },
        }}),
    )
}
fn serialize(value: serde_json::Value) -> Result<String> {
    let serialized = serde_json::to_string(&value).context("could not serialize MCP response")?;
    (serialized.len() <= MAX_MCP_SERIALIZED_RESULT_BYTES)
        .then_some(serialized)
        .ok_or_else(|| anyhow::anyhow!("limit_exceeded"))
}
fn tool_response(response: Result<String>) -> String {
    response.unwrap_or_else(|error| {
        let code = error.to_string();
        json!({"error": if is_public_code(&code) { code.as_str() } else { "operation_failed" }})
            .to_string()
    })
}
fn is_public_code(value: &str) -> bool {
    matches!(
        value,
        "invalid_request"
            | "invalid_definition"
            | "invalid_reference"
            | "reference_unavailable"
            | "not_found"
            | "builtin_protected"
            | "unknown_tool"
            | "tool_disallowed"
            | "memory_denied"
            | "knowledge_denied"
            | "sandbox_denied"
            | "adapter_failure"
            | "limit_exceeded"
            | "cancelled"
            | "deadline_exceeded"
            | "operation_failed"
    )
}
const fn public_code(code: PublicErrorCode) -> &'static str {
    match code {
        PublicErrorCode::InvalidRequest => "invalid_request",
        PublicErrorCode::InvalidDefinition => "invalid_definition",
        PublicErrorCode::InvalidReference => "invalid_reference",
        PublicErrorCode::ReferenceUnavailable => "reference_unavailable",
        PublicErrorCode::NotFound => "not_found",
        PublicErrorCode::BuiltinProtected => "builtin_protected",
        PublicErrorCode::UnknownTool => "unknown_tool",
        PublicErrorCode::ToolDisallowed => "tool_disallowed",
        PublicErrorCode::MemoryDenied => "memory_denied",
        PublicErrorCode::KnowledgeDenied => "knowledge_denied",
        PublicErrorCode::SandboxDenied => "sandbox_denied",
        PublicErrorCode::AdapterFailure => "adapter_failure",
        PublicErrorCode::LimitExceeded => "limit_exceeded",
        PublicErrorCode::Cancelled => "cancelled",
        PublicErrorCode::DeadlineExceeded => "deadline_exceeded",
    }
}
#[must_use]
pub const fn tool_names() -> [&'static str; 5] {
    AGENT_DEFINITION_TOOLS
}

#[cfg(test)]
mod migration_tests {
    use std::{
        future::Future,
        pin::Pin,
        sync::{Arc, Mutex},
        task::{Context, Poll},
        time::Instant,
    };

    use llm_gateway::{
        CancellationFuture, CancellationSignal, DeadlineFuture, DeadlineSignal, FinishReason,
        IdempotencyDisposition, IdempotencyKey, InvocationControl, LlmProvider, ProviderFuture,
        r#static::{StaticFixture, StaticProvider},
    };
    use policy::{
        AuthorizationDecisionV1, AuthorizationRequestV1, CorrelationId as PolicyCorrelationId,
        GrantV1, PolicyResolver, PrincipalId as PolicyPrincipalId, RequestId as PolicyRequestId,
        TenantId as PolicyTenantId, TrustedContextV1, allow_decision, deny_decision,
    };

    use super::*;
    use crate::{
        AgentDefinitionV1, CommunicationPolicy, DefinitionVersion, DenySandbox, FixedToolRegistry,
        InMemoryDefinitionStore, InMemoryMemoryStore, KnowledgePolicy, MemoryPolicy, ModelPolicy,
        SandboxPolicy, StaticReferenceCatalog,
    };

    struct Cancellation;
    impl CancellationSignal for Cancellation {
        fn is_cancelled(&self) -> bool {
            false
        }
        fn cancelled(&self) -> CancellationFuture<'_> {
            Box::pin(std::future::pending())
        }
    }
    struct Deadline;
    impl DeadlineSignal for Deadline {
        fn instant(&self) -> Instant {
            Instant::now()
        }
        fn is_elapsed(&self) -> bool {
            false
        }
        fn elapsed(&self) -> DeadlineFuture<'_> {
            Box::pin(std::future::pending())
        }
    }
    struct Bundle {
        key: IdempotencyKey,
        cancellation: Cancellation,
        deadline: Deadline,
    }
    impl InvocationControlBundle for Bundle {
        fn control(&self) -> InvocationControl<'_> {
            InvocationControl {
                idempotency_key: &self.key,
                cancellation: &self.cancellation,
                deadline: &self.deadline,
            }
        }
    }
    struct ControlSource(Arc<Mutex<u32>>);
    impl InvocationControlSource for ControlSource {
        fn create(&self) -> std::result::Result<Box<dyn InvocationControlBundle>, DefinitionError> {
            *self.0.lock().expect("control calls") += 1;
            Ok(Box::new(Bundle {
                key: IdempotencyKey::new("mcp-attempt").expect("key"),
                cancellation: Cancellation,
                deadline: Deadline,
            }))
        }
    }

    #[derive(Clone)]
    struct ContextSource;
    impl TrustedContextSource for ContextSource {
        fn resolve(&self) -> std::result::Result<TrustedContextV1, DefinitionError> {
            Ok(TrustedContextV1 {
                tenant_id: PolicyTenantId::new("host-tenant").expect("tenant"),
                principal_id: PolicyPrincipalId::new("host-principal").expect("principal"),
                request_id: PolicyRequestId::new("host-request").expect("request"),
                correlation_id: PolicyCorrelationId::new("host-correlation").expect("correlation"),
            })
        }
    }
    #[derive(Clone)]
    struct Policy(bool);
    impl PolicyResolver for Policy {
        fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
            if self.0 {
                allow_decision(
                    &request,
                    &GrantV1::new(Vec::<String>::new(), false, false, false, false).expect("grant"),
                )
                .expect("decision")
            } else {
                deny_decision()
            }
        }
    }

    fn poll_ready<T>(future: impl Future<Output = T>) -> T {
        let mut context = Context::from_waker(std::task::Waker::noop());
        let mut future = Box::pin(future);
        match Future::poll(Pin::as_mut(&mut future), &mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("static invocation unexpectedly pending"),
        }
    }

    fn definition() -> AgentDefinitionV1 {
        AgentDefinitionV1 {
            version: DefinitionVersion::V1,
            id: AgentId::new("agent").expect("id"),
            name: "Agent".to_owned(),
            description: "MCP migration fixture".to_owned(),
            model: ModelPolicy {
                reference: "static.model".to_owned(),
            },
            instructions: "Respond.".to_owned(),
            skills: vec![],
            steering: vec![],
            allowed_tool_ids: vec![],
            memory: MemoryPolicy {
                enabled: false,
                max_items: 0,
            },
            knowledge: KnowledgePolicy {
                enabled: false,
                namespace: "default".to_owned(),
                max_results: 0,
            },
            sandbox: SandboxPolicy {
                allow_execution: false,
            },
            communication: CommunicationPolicy {
                allow_messages: false,
            },
            limits: ExecutionLimits {
                max_tool_calls: 1,
                max_output_bytes: 128,
            },
        }
    }

    fn service(
        allow: bool,
        control_calls: Arc<Mutex<u32>>,
    ) -> AgentDefinitionMcp<
        InMemoryDefinitionStore,
        StaticReferenceCatalog,
        StaticProvider,
        FixedToolRegistry,
        InMemoryMemoryStore,
        knowledge::r#static::StaticKnowledgeIndex,
        DenySandbox,
        ContextSource,
        Policy,
    > {
        AgentDefinitionMcp::new(
            vec![definition()],
            InMemoryDefinitionStore::default(),
            StaticReferenceCatalog::new(["static.model".to_owned()], [], [], []),
            StaticProvider::success(
                StaticFixture::new(
                    "ok",
                    vec![],
                    None,
                    FinishReason::Stop,
                    None,
                    IdempotencyDisposition::Unsupported,
                )
                .expect("fixture"),
            ),
            FixedToolRegistry::default(),
            InMemoryMemoryStore::default(),
            knowledge::r#static::StaticKnowledgeIndex::new(vec![]).expect("knowledge"),
            DenySandbox,
            AgentPolicyContextResolver::new(ContextSource, Policy(allow)),
            Box::new(ControlSource(control_calls)),
        )
        .expect("service")
    }

    #[test]
    fn mcp_awaits_host_owned_controls_only_after_policy_allows() {
        let denied_calls = Arc::new(Mutex::new(0));
        let denied = service(false, Arc::clone(&denied_calls));
        assert_eq!(
            tool_response(poll_ready(denied.invoke_json(InvokeInput {
                id: "agent".to_owned(),
                input: String::new(),
            }))),
            "{\"error\":\"not_found\"}"
        );
        assert_eq!(*denied_calls.lock().expect("calls"), 0);

        let allowed_calls = Arc::new(Mutex::new(0));
        let allowed = service(true, Arc::clone(&allowed_calls));
        let result = poll_ready(allowed.invoke_json(InvokeInput {
            id: "agent".to_owned(),
            input: String::new(),
        }))
        .expect("invoke");
        assert!(result.contains("\"output\":\"ok\""));
        assert!(result.contains("\"provider_id\":\"static\""));
        assert_eq!(*allowed_calls.lock().expect("calls"), 1);
    }

    #[test]
    fn invoke_ingress_is_closed_and_oversized_input_is_pre_policy_and_pre_control() {
        assert!(
            serde_json::from_value::<InvokeInput>(json!({
                "id": "agent",
                "input": "request",
                "tenant_id": "caller-controlled"
            }))
            .is_err()
        );
        let schema = serde_json::to_value(schemars::schema_for!(InvokeInput)).expect("schema");
        assert_eq!(schema["additionalProperties"], false);

        let control_calls = Arc::new(Mutex::new(0));
        let server = service(true, Arc::clone(&control_calls));
        let result = tool_response(poll_ready(server.invoke_json(InvokeInput {
            id: "agent".to_owned(),
            input: "x".repeat(MAX_INPUT_BYTES + 1),
        })));
        assert_eq!(result, "{\"error\":\"limit_exceeded\"}");
        assert_eq!(*control_calls.lock().expect("calls"), 0);
    }

    #[test]
    fn invoke_output_is_an_exact_safe_projection() {
        let server = service(true, Arc::new(Mutex::new(0)));
        let serialized = poll_ready(server.invoke_json(InvokeInput {
            id: "agent".to_owned(),
            input: "request".to_owned(),
        }))
        .expect("invoke");
        let value: serde_json::Value = serde_json::from_str(&serialized).expect("json");
        let root = value.as_object().expect("object");
        assert_eq!(
            root.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "capability_scope_digest",
                "events",
                "model_evidence",
                "output"
            ]
        );
        let evidence = root["model_evidence"].as_object().expect("evidence");
        assert_eq!(
            evidence.keys().map(String::as_str).collect::<Vec<_>>(),
            [
                "finish_reason",
                "idempotency",
                "model_id",
                "provider_id",
                "provider_request_id",
                "token_usage"
            ]
        );
        for forbidden in [
            "credential",
            "endpoint",
            "headers",
            "raw_error",
            "prompt",
            "arguments",
            "tenant_id",
            "principal_id",
        ] {
            assert!(
                !serialized.contains(forbidden),
                "MCP output leaked {forbidden}"
            );
        }
    }

    struct PendingModel {
        calls: Arc<Mutex<u32>>,
        dropped: Arc<Mutex<bool>>,
    }
    impl LlmProvider for PendingModel {
        fn generate<'a>(
            &'a self,
            _: &'a llm_gateway::GenerateRequest,
            _: InvocationControl<'a>,
        ) -> ProviderFuture<'a> {
            *self.calls.lock().expect("provider calls") += 1;
            Box::pin(PendingModelFuture {
                dropped: Arc::clone(&self.dropped),
            })
        }
    }
    struct PendingModelFuture {
        dropped: Arc<Mutex<bool>>,
    }
    impl Future for PendingModelFuture {
        type Output = std::result::Result<llm_gateway::GenerateResponse, llm_gateway::LlmError>;
        fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
            Poll::Pending
        }
    }
    impl Drop for PendingModelFuture {
        fn drop(&mut self) {
            *self.dropped.lock().expect("dropped") = true;
        }
    }

    #[test]
    fn mcp_policy_precedes_control_and_provider_and_awaits_without_blocking() {
        let provider_calls = Arc::new(Mutex::new(0));
        let dropped = Arc::new(Mutex::new(false));
        let control_calls = Arc::new(Mutex::new(0));
        let server = AgentDefinitionMcp::new(
            vec![definition()],
            InMemoryDefinitionStore::default(),
            StaticReferenceCatalog::new(["static.model".to_owned()], [], [], []),
            PendingModel {
                calls: Arc::clone(&provider_calls),
                dropped: Arc::clone(&dropped),
            },
            FixedToolRegistry::default(),
            InMemoryMemoryStore::default(),
            knowledge::r#static::StaticKnowledgeIndex::new(vec![]).expect("knowledge"),
            DenySandbox,
            AgentPolicyContextResolver::new(ContextSource, Policy(true)),
            Box::new(ControlSource(Arc::clone(&control_calls))),
        )
        .expect("service");
        let mut future = Box::pin(server.invoke_json(InvokeInput {
            id: "agent".to_owned(),
            input: String::new(),
        }));
        let mut context = Context::from_waker(std::task::Waker::noop());
        assert!(Future::poll(future.as_mut(), &mut context).is_pending());
        assert_eq!(*control_calls.lock().expect("control calls"), 1);
        assert_eq!(*provider_calls.lock().expect("provider calls"), 1);
        drop(future);
        assert!(*dropped.lock().expect("dropped"));
    }

    struct CapturingMemory(Arc<Mutex<Option<InvocationContextV1>>>);
    impl MemoryStore for CapturingMemory {
        fn recall(
            &self,
            request: crate::MemoryRequest,
        ) -> std::result::Result<Vec<String>, DefinitionError> {
            *self.0.lock().expect("context") = Some(request.context);
            Ok(vec![])
        }

        fn write(
            &self,
            request: crate::MemoryRequest,
            _: String,
        ) -> std::result::Result<(), DefinitionError> {
            *self.0.lock().expect("context") = Some(request.context);
            Ok(())
        }
    }

    #[derive(Clone)]
    struct MemoryPolicyResolver;
    impl PolicyResolver for MemoryPolicyResolver {
        fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
            allow_decision(
                &request,
                &GrantV1::new(Vec::<String>::new(), true, false, false, false).expect("grant"),
            )
            .expect("decision")
        }
    }

    #[test]
    fn mcp_host_identity_reaches_effect_and_caller_identity_is_rejected() {
        assert!(
            serde_json::from_value::<InvokeInput>(json!({
                "id": "agent",
                "input": "request",
                "principal_id": "caller-principal"
            }))
            .is_err()
        );
        let captured = Arc::new(Mutex::new(None));
        let mut value = definition();
        value.memory = MemoryPolicy {
            enabled: true,
            max_items: 1,
        };
        let provider = StaticProvider::success(
            StaticFixture::new(
                "ok",
                vec![
                    llm_gateway::ToolCall::new(
                        llm_gateway::ToolName::new("factory.memory.write").expect("name"),
                        llm_gateway::JsonObject::new(r#"{"value":"remember"}"#).expect("arguments"),
                    )
                    .expect("call"),
                ],
                None,
                FinishReason::ToolCalls,
                None,
                IdempotencyDisposition::Unsupported,
            )
            .expect("fixture"),
        );
        let server = AgentDefinitionMcp::new(
            vec![value],
            InMemoryDefinitionStore::default(),
            StaticReferenceCatalog::new(["static.model".to_owned()], [], [], []),
            provider,
            FixedToolRegistry::default(),
            CapturingMemory(Arc::clone(&captured)),
            knowledge::r#static::StaticKnowledgeIndex::new(vec![]).expect("knowledge"),
            DenySandbox,
            AgentPolicyContextResolver::new(ContextSource, MemoryPolicyResolver),
            Box::new(ControlSource(Arc::new(Mutex::new(0)))),
        )
        .expect("service");
        poll_ready(server.invoke_json(InvokeInput {
            id: "agent".to_owned(),
            input: String::new(),
        }))
        .expect("invoke");
        let context = captured.lock().expect("context").clone().expect("captured");
        assert_eq!(context.tenant_id().as_str(), "host-tenant");
        assert_eq!(context.principal_id().as_str(), "host-principal");
        assert_eq!(context.request_id().as_str(), "host-request");
        assert_eq!(context.correlation_id().as_str(), "host-correlation");
    }

    #[test]
    fn definition_schema_requires_closed_knowledge_namespace_and_projects_it() {
        let schema = serde_json::to_value(schemars::schema_for!(AgentDefinitionInput))
            .expect("definition schema");
        assert_eq!(schema["additionalProperties"], false);
        assert!(
            schema["required"]
                .as_array()
                .expect("required")
                .iter()
                .any(|field| field == "knowledge_namespace")
        );

        let mut input = json!({
            "id": "agent",
            "name": "Agent",
            "description": "Definition",
            "model_reference": "static.model",
            "instructions": "Respond.",
            "memory_enabled": false,
            "memory_max_items": 0,
            "knowledge_enabled": false,
            "knowledge_namespace": "default",
            "knowledge_max_results": 0,
            "sandbox_allow_execution": false,
            "communication_allow_messages": false,
            "max_tool_calls": 1,
            "max_output_bytes": 128
        });
        let converted = serde_json::from_value::<AgentDefinitionInput>(input.clone())
            .expect("definition input")
            .into_core()
            .expect("core definition");
        assert_eq!(
            converted.knowledge,
            KnowledgePolicy {
                enabled: false,
                namespace: "default".to_owned(),
                max_results: 0,
            }
        );
        input
            .as_object_mut()
            .expect("object")
            .remove("knowledge_namespace");
        assert!(serde_json::from_value::<AgentDefinitionInput>(input).is_err());

        let projected: serde_json::Value =
            serde_json::from_str(&definition_json(&definition()).expect("definition projection"))
                .expect("definition json");
        assert_eq!(
            projected["knowledge"],
            json!({"enabled": false, "namespace": "default", "max_results": 0})
        );
    }

    #[test]
    fn knowledge_event_projection_is_exact_and_context_free() {
        let serialized = invocation_json(crate::InvocationResult {
            capability_scope_digest: "digest".to_owned(),
            events: vec![crate::InvocationEvent::KnowledgeSearched {
                results: vec![crate::KnowledgeResult {
                    document_id: "doc-1".to_owned(),
                    text: "bounded text".to_owned(),
                }],
            }],
            output: String::new(),
            model_evidence: crate::InvocationModelEvidence {
                provider_id: "static".to_owned(),
                model_id: "model".to_owned(),
                provider_request_id: None,
                finish_reason: crate::InvocationModelFinishReason::Stop,
                token_usage: None,
                idempotency: crate::InvocationModelIdempotency::Unsupported,
            },
        })
        .expect("projection");
        let value: serde_json::Value = serde_json::from_str(&serialized).expect("json");
        assert_eq!(
            value["events"][0],
            json!({
                "type": "knowledge_searched",
                "results": [{"document_id": "doc-1", "text": "bounded text"}]
            })
        );
        for forbidden in ["tenant_id", "principal_id", "namespace"] {
            assert!(!serialized.contains(forbidden));
        }
    }
}
