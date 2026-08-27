#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::needless_pass_by_value)]

//! Bounded MCP control-plane adapter for agent definitions and local invocation.

use agent_core::{
    AgentDefinitionV1, AgentId, AgentRegistry, CommunicationPolicy, DefinitionError,
    DefinitionStore, DefinitionVersion, EffectiveCapabilityCeilingV1, ExecutionLimits,
    KnowledgePolicy, KnowledgeStore, LocalAgentRuntime, MAX_INPUT_BYTES, MemoryPolicy, MemoryStore,
    ModelPolicy, ModelProvider, PublicErrorCode, ReferenceCatalog, Sandbox, SandboxPolicy,
    ToolRegistry, validate_definition,
};
use anyhow::{Context, Result};
use mcp_transport::BoundedStdioTransport;
use policy_core::{
    AuthorizationDecisionV1, AuthorizationRequestV1, CapabilityV1, GrantV1, PolicyResolver,
    TrustedContextV1, canonical_grant, decision_digest,
};
use rmcp::{
    ServerHandler, ServiceExt,
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

#[derive(Clone, Copy)]
enum PolicyGateError {
    Denied,
    Failed,
}

struct AuthorizedAgentContext {
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
        Ok(AuthorizedAgentContext { grant })
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
    M: ModelProvider,
    T: ToolRegistry,
    MM: MemoryStore,
    K: KnowledgeStore,
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
    tool_router: ToolRouter<Self>,
}
impl<S, C, M, T, MM, K, SB, TS, P> AgentDefinitionMcp<S, C, M, T, MM, K, SB, TS, P>
where
    S: DefinitionStore + 'static,
    C: ReferenceCatalog + 'static,
    M: ModelProvider + 'static,
    T: ToolRegistry + 'static,
    MM: MemoryStore + 'static,
    K: KnowledgeStore + 'static,
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
    ) -> std::result::Result<Self, DefinitionError> {
        Ok(Self {
            registry: AgentRegistry::new(builtins, store, catalog)?,
            model,
            tools,
            memory,
            knowledge,
            sandbox,
            resolver,
            tool_router: Self::tool_router(),
        })
    }

    pub async fn serve_stdio(self) -> Result<()> {
        self.serve(BoundedStdioTransport::new(
            tokio::io::stdin(),
            tokio::io::stdout(),
        ))
        .await?
        .waiting()
        .await?;
        Ok(())
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

    fn invoke_json(&self, input: InvokeInput) -> Result<String> {
        let id = AgentId::new(input.id).map_err(|_| anyhow::anyhow!("invalid_definition"))?;
        if input.input.len() > MAX_INPUT_BYTES {
            return Err(anyhow::anyhow!("limit_exceeded"));
        }
        let authorized = self
            .resolver
            .resolve_and_authorize(CapabilityV1::AgentInvoke)
            .map_err(policy_error)?;
        let ceiling = EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: authorized.grant.allowed_tool_ids,
            memory_enabled: authorized.grant.memory_enabled,
            knowledge_enabled: authorized.grant.knowledge_enabled,
            sandbox_execution_allowed: authorized.grant.sandbox_execution_allowed,
            communication_allowed: authorized.grant.communication_allowed,
        };
        let runtime = LocalAgentRuntime::new(
            &self.registry,
            &self.model,
            &self.tools,
            &self.memory,
            &self.knowledge,
            &self.sandbox,
        );
        let result = runtime
            .invoke_with_ceiling(&id, input.input, &ceiling)
            .map_err(domain_error)?;
        invocation_json(result)
    }
}

#[tool_router(router = tool_router)]
impl<S, C, M, T, MM, K, SB, TS, P> AgentDefinitionMcp<S, C, M, T, MM, K, SB, TS, P>
where
    S: DefinitionStore + 'static,
    C: ReferenceCatalog + 'static,
    M: ModelProvider + 'static,
    T: ToolRegistry + 'static,
    MM: MemoryStore + 'static,
    K: KnowledgeStore + 'static,
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
        tool_response(self.invoke_json(input))
    }
}

#[tool_handler(router = self.tool_router)]
impl<S, C, M, T, MM, K, SB, TS, P> ServerHandler
    for AgentDefinitionMcp<S, C, M, T, MM, K, SB, TS, P>
where
    S: DefinitionStore + 'static,
    C: ReferenceCatalog + 'static,
    M: ModelProvider + 'static,
    T: ToolRegistry + 'static,
    MM: MemoryStore + 'static,
    K: KnowledgeStore + 'static,
    SB: Sandbox + 'static,
    TS: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
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
        json!({"version": definition.version.as_str(), "id": definition.id.as_str(), "name": definition.name, "description": definition.description, "model_reference": definition.model.reference, "instructions": definition.instructions, "skills": definition.skills, "steering": definition.steering, "allowed_tool_ids": definition.allowed_tool_ids, "memory": {"enabled": definition.memory.enabled, "max_items": definition.memory.max_items}, "knowledge": {"enabled": definition.knowledge.enabled, "max_results": definition.knowledge.max_results}, "sandbox": {"allow_execution": definition.sandbox.allow_execution}, "communication": {"allow_messages": definition.communication.allow_messages}, "limits": {"max_tool_calls": definition.limits.max_tool_calls, "max_output_bytes": definition.limits.max_output_bytes}}),
    )
}
fn invocation_json(result: agent_core::InvocationResult) -> Result<String> {
    serialize(
        json!({"capability_scope_digest": result.capability_scope_digest, "events": result.events.into_iter().map(|event| match event {
        agent_core::InvocationEvent::ModelInvoked => json!({"type":"model_invoked"}),
        agent_core::InvocationEvent::MemoryRecalled { values } => json!({"type":"memory_recalled", "values":values}),
        agent_core::InvocationEvent::MemoryWritten => json!({"type":"memory_written"}),
        agent_core::InvocationEvent::KnowledgeSearched { results } => json!({"type":"knowledge_searched", "results":results}),
        agent_core::InvocationEvent::SandboxCompleted { output } => json!({"type":"sandbox_completed", "output":output}),
        agent_core::InvocationEvent::ToolCompleted { tool_id, output } => json!({"type":"tool_completed", "tool_id":tool_id, "output":output}),
    }).collect::<Vec<_>>(), "output": result.output}),
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
        "invalid_definition"
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
            | "operation_failed"
    )
}
const fn public_code(code: PublicErrorCode) -> &'static str {
    match code {
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
    }
}
#[must_use]
pub const fn tool_names() -> [&'static str; 5] {
    AGENT_DEFINITION_TOOLS
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use agent_core::{
        DenySandbox, FixedToolRegistry, InMemoryDefinitionStore, InMemoryMemoryStore,
        ModelResponse, StaticKnowledgeStore, StaticModelProvider, StaticReferenceCatalog,
    };
    use policy_core::{
        CorrelationId, PrincipalId, RequestId, TenantId, allow_decision, deny_decision,
    };

    use super::*;

    #[derive(Clone)]
    struct Source {
        context: std::result::Result<TrustedContextV1, DefinitionError>,
        calls: Arc<Mutex<u32>>,
    }
    impl TrustedContextSource for Source {
        fn resolve(&self) -> std::result::Result<TrustedContextV1, DefinitionError> {
            *self.calls.lock().expect("source calls") += 1;
            self.context.clone()
        }
    }

    #[derive(Clone)]
    struct Policy {
        allow: bool,
        tamper: bool,
        grant: Option<GrantV1>,
        calls: Arc<Mutex<Vec<CapabilityV1>>>,
    }
    impl PolicyResolver for Policy {
        fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
            self.calls
                .lock()
                .expect("policy calls")
                .push(request.capability);
            if !self.allow {
                return deny_decision();
            }
            let AuthorizationDecisionV1::Allow {
                effective_grant,
                decision_digest,
            } = allow_decision(
                &request,
                &self.grant.clone().unwrap_or_else(|| {
                    GrantV1::new(Vec::<String>::new(), false, false, false, false).expect("grant")
                }),
            )
            .expect("allow decision")
            else {
                unreachable!();
            };
            AuthorizationDecisionV1::Allow {
                effective_grant,
                decision_digest: if self.tamper {
                    "0".repeat(64)
                } else {
                    decision_digest
                },
            }
        }
    }

    type TestMcp = AgentDefinitionMcp<
        InMemoryDefinitionStore,
        StaticReferenceCatalog,
        StaticModelProvider,
        FixedToolRegistry,
        InMemoryMemoryStore,
        StaticKnowledgeStore,
        DenySandbox,
        Source,
        Policy,
    >;

    fn trusted() -> TrustedContextV1 {
        TrustedContextV1 {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            principal_id: PrincipalId::new("principal").expect("principal"),
            request_id: RequestId::new("request").expect("request"),
            correlation_id: CorrelationId::new("correlation").expect("correlation"),
        }
    }
    fn definition() -> AgentDefinitionInput {
        AgentDefinitionInput {
            id: "agent".to_owned(),
            name: "Agent".to_owned(),
            description: "Test agent".to_owned(),
            model_reference: "static".to_owned(),
            instructions: "Respond.".to_owned(),
            skills: vec![],
            steering: vec![],
            allowed_tool_ids: vec![],
            memory_enabled: false,
            memory_max_items: 0,
            knowledge_enabled: false,
            knowledge_max_results: 0,
            sandbox_allow_execution: false,
            communication_allow_messages: false,
            max_tool_calls: 1,
            max_output_bytes: 10,
        }
    }
    type SourceCalls = Arc<Mutex<u32>>;
    type PolicyCalls = Arc<Mutex<Vec<CapabilityV1>>>;
    type SchemaCase = (
        &'static str,
        serde_json::Value,
        fn(serde_json::Value) -> bool,
        serde_json::Value,
    );

    fn service(source_ok: bool, allow: bool, tamper: bool) -> (TestMcp, SourceCalls, PolicyCalls) {
        service_with_context(trusted(), source_ok, allow, tamper)
    }
    fn service_with_context(
        context: TrustedContextV1,
        source_ok: bool,
        allow: bool,
        tamper: bool,
    ) -> (TestMcp, SourceCalls, PolicyCalls) {
        let source_calls = Arc::new(Mutex::new(0));
        let policy_calls = Arc::new(Mutex::new(vec![]));
        let resolver = AgentPolicyContextResolver::new(
            Source {
                context: if source_ok {
                    Ok(context)
                } else {
                    Err(DefinitionError::AdapterFailure)
                },
                calls: Arc::clone(&source_calls),
            },
            Policy {
                allow,
                tamper,
                grant: None,
                calls: Arc::clone(&policy_calls),
            },
        );
        let server = TestMcp::new(
            vec![definition().clone().into_core().expect("definition")],
            InMemoryDefinitionStore::default(),
            StaticReferenceCatalog::new(["static".to_owned()], [], [], []),
            StaticModelProvider::new(ModelResponse {
                output: "ok".to_owned(),
                tool_calls: vec![],
                capability_requests: vec![],
            }),
            FixedToolRegistry::default(),
            InMemoryMemoryStore::default(),
            StaticKnowledgeStore::default(),
            DenySandbox,
            resolver,
        )
        .expect("server");
        (server, source_calls, policy_calls)
    }
    #[derive(Clone, Copy)]
    enum Operation {
        Validate,
        Get,
        List,
        Register,
        Invoke,
    }
    impl Operation {
        const fn capability(self) -> CapabilityV1 {
            match self {
                Self::Validate => CapabilityV1::AgentDefinitionValidate,
                Self::Get => CapabilityV1::AgentDefinitionGet,
                Self::List => CapabilityV1::AgentDefinitionList,
                Self::Register => CapabilityV1::AgentDefinitionRegister,
                Self::Invoke => CapabilityV1::AgentInvoke,
            }
        }
    }
    fn call(server: &TestMcp, operation: Operation) -> String {
        match operation {
            Operation::Validate => server.validate_json(definition()).expect("validate"),
            Operation::Get => tool_response(server.get_json(AgentIdInput {
                id: "agent".to_owned(),
            })),
            Operation::List => tool_response(server.list_json()),
            Operation::Register => tool_response(server.register_json(AgentDefinitionInput {
                id: "registered".to_owned(),
                ..definition()
            })),
            Operation::Invoke => tool_response(server.invoke_json(InvokeInput {
                id: "agent".to_owned(),
                input: String::new(),
            })),
        }
    }

    #[test]
    fn policy_gate_maps_failures_and_authorizes_exact_capabilities() {
        for (source_ok, allow, tamper, expected) in [
            (false, true, false, "operation_failed"),
            (true, false, false, "not_found"),
            (true, true, true, "operation_failed"),
        ] {
            for operation in [
                Operation::Validate,
                Operation::Get,
                Operation::List,
                Operation::Register,
                Operation::Invoke,
            ] {
                let (server, source_calls, policy_calls) = service(source_ok, allow, tamper);
                assert!(call(&server, operation).contains(expected));
                assert_eq!(*source_calls.lock().expect("source calls"), 1);
                let calls = policy_calls.lock().expect("policy calls");
                if source_ok {
                    assert_eq!(calls.as_slice(), &[operation.capability()]);
                } else {
                    assert!(calls.is_empty());
                }
            }
        }
    }

    #[test]
    fn malformed_or_oversized_input_is_pre_policy() {
        let (server, source_calls, policy_calls) = service(true, true, false);
        assert!(
            server
                .get_json(AgentIdInput {
                    id: "Invalid".to_owned()
                })
                .is_err()
        );
        assert_eq!(*source_calls.lock().expect("source calls"), 0);
        assert!(policy_calls.lock().expect("policy calls").is_empty());
        assert_eq!(
            tool_response(server.invoke_json(InvokeInput {
                id: "agent".to_owned(),
                input: "x".repeat(MAX_INPUT_BYTES + 1)
            })),
            "{\"error\":\"limit_exceeded\"}"
        );
        assert_eq!(*source_calls.lock().expect("source calls"), 0);
        assert!(policy_calls.lock().expect("policy calls").is_empty());
    }

    #[test]
    fn prohibited_mcp_fields_are_rejected_by_every_typed_parameter_schema_pre_policy() {
        // `#[tool]` generates private parameter extraction. These serde/schema checks cover each
        // generated handler's typed input, including the zero-argument list input, before a
        // handler can resolve trusted context, policy, or an injected domain port.
        let prohibited = [
            "tenant_id",
            "principal_id",
            "request_id",
            "correlation_id",
            "grant",
            "decision_digest",
            "effective_capability_ceiling",
        ];
        let cases: [SchemaCase; 5] = [
            (
                "validate",
                serde_json::to_value(definition()).expect("definition JSON"),
                |value| serde_json::from_value::<AgentDefinitionInput>(value).is_err(),
                serde_json::to_value(schemars::schema_for!(AgentDefinitionInput)).expect("schema"),
            ),
            (
                "get",
                json!({"id":"agent"}),
                |value| serde_json::from_value::<AgentIdInput>(value).is_err(),
                serde_json::to_value(schemars::schema_for!(AgentIdInput)).expect("schema"),
            ),
            (
                "list",
                json!({}),
                |value| serde_json::from_value::<ListInput>(value).is_err(),
                serde_json::to_value(schemars::schema_for!(ListInput)).expect("schema"),
            ),
            (
                "register",
                serde_json::to_value(definition()).expect("definition JSON"),
                |value| serde_json::from_value::<AgentDefinitionInput>(value).is_err(),
                serde_json::to_value(schemars::schema_for!(AgentDefinitionInput)).expect("schema"),
            ),
            (
                "invoke",
                json!({"id":"agent","input":"request"}),
                |value| serde_json::from_value::<InvokeInput>(value).is_err(),
                serde_json::to_value(schemars::schema_for!(InvokeInput)).expect("schema"),
            ),
        ];
        for (operation, baseline, parse_rejects, schema) in cases {
            assert_eq!(schema["additionalProperties"], false, "{operation} schema");
            for field in prohibited {
                let mut payload = baseline.clone();
                payload
                    .as_object_mut()
                    .expect("parameter object")
                    .insert(field.to_owned(), json!("untrusted"));
                assert!(parse_rejects(payload), "{operation} accepted {field}");
            }
        }
    }

    #[derive(Clone, Default)]
    struct PortCalls(Arc<Mutex<Vec<&'static str>>>);
    impl PortCalls {
        fn record(&self, operation: &'static str) {
            self.0.lock().expect("port calls").push(operation);
        }
        fn assert_empty(&self) {
            assert!(self.0.lock().expect("port calls").is_empty());
        }
        fn clear(&self) {
            self.0.lock().expect("port calls").clear();
        }
    }

    struct RecordingStore(PortCalls);
    impl DefinitionStore for RecordingStore {
        fn load(
            &self,
            _: &AgentId,
        ) -> std::result::Result<Option<AgentDefinitionV1>, DefinitionError> {
            self.0.record("store.load");
            Ok(None)
        }
        fn list(&self) -> std::result::Result<Vec<AgentDefinitionV1>, DefinitionError> {
            self.0.record("store.list");
            Ok(vec![])
        }
        fn save(&self, _: AgentDefinitionV1) -> std::result::Result<(), DefinitionError> {
            self.0.record("store.save");
            Ok(())
        }
        fn delete(&self, _: &AgentId) -> std::result::Result<(), DefinitionError> {
            self.0.record("store.delete");
            Ok(())
        }
    }
    struct RecordingCatalog(PortCalls);
    impl ReferenceCatalog for RecordingCatalog {
        fn contains_model(&self, _: &str) -> std::result::Result<bool, DefinitionError> {
            self.0.record("catalog.model");
            Ok(true)
        }
        fn contains_skill(&self, _: &str) -> std::result::Result<bool, DefinitionError> {
            self.0.record("catalog.skill");
            Ok(true)
        }
        fn contains_steering(&self, _: &str) -> std::result::Result<bool, DefinitionError> {
            self.0.record("catalog.steering");
            Ok(true)
        }
        fn contains_tool(&self, _: &str) -> std::result::Result<bool, DefinitionError> {
            self.0.record("catalog.tool");
            Ok(true)
        }
    }
    struct RecordingModel {
        calls: PortCalls,
        responses: Mutex<Vec<ModelResponse>>,
    }
    impl ModelProvider for RecordingModel {
        fn invoke(
            &self,
            request: agent_core::ModelRequest,
        ) -> std::result::Result<ModelResponse, DefinitionError> {
            self.calls.record("model.invoke");
            MODEL_SCOPES.with(|scopes| {
                scopes
                    .lock()
                    .expect("model scopes")
                    .push(request.capability_scope);
            });
            Ok(self.responses.lock().expect("responses").remove(0))
        }
    }
    thread_local! { static MODEL_SCOPES: Mutex<Vec<agent_core::ResolvedCapabilityScope>> = const { Mutex::new(Vec::new()) }; }
    struct RecordingTools(PortCalls);
    impl ToolRegistry for RecordingTools {
        fn resolve(
            &self,
            id: &str,
        ) -> std::result::Result<agent_core::ToolDescriptor, DefinitionError> {
            self.0.record("tools.resolve");
            Ok(agent_core::ToolDescriptor { id: id.to_owned() })
        }
        fn invoke(
            &self,
            _: &agent_core::ToolDescriptor,
            _: agent_core::ToolRequest,
        ) -> std::result::Result<String, DefinitionError> {
            self.0.record("tools.invoke");
            Ok("tool output".to_owned())
        }
    }
    struct RecordingMemory(PortCalls);
    impl MemoryStore for RecordingMemory {
        fn recall(
            &self,
            _: agent_core::MemoryRequest,
        ) -> std::result::Result<Vec<String>, DefinitionError> {
            self.0.record("memory.recall");
            Ok(vec![])
        }
        fn write(
            &self,
            _: agent_core::MemoryRequest,
            _: String,
        ) -> std::result::Result<(), DefinitionError> {
            self.0.record("memory.write");
            Ok(())
        }
    }
    struct RecordingKnowledge(PortCalls);
    impl KnowledgeStore for RecordingKnowledge {
        fn search(
            &self,
            _: agent_core::KnowledgeRequest,
        ) -> std::result::Result<Vec<String>, DefinitionError> {
            self.0.record("knowledge.search");
            Ok(vec![])
        }
    }
    struct RecordingSandbox(PortCalls);
    impl Sandbox for RecordingSandbox {
        fn execute(
            &self,
            _: agent_core::SandboxRequest,
        ) -> std::result::Result<String, DefinitionError> {
            self.0.record("sandbox.execute");
            Ok("sandbox output".to_owned())
        }
    }

    type RecordingMcp = AgentDefinitionMcp<
        RecordingStore,
        RecordingCatalog,
        RecordingModel,
        RecordingTools,
        RecordingMemory,
        RecordingKnowledge,
        RecordingSandbox,
        Source,
        Policy,
    >;
    fn recording_service(
        source_ok: bool,
        allow: bool,
        tamper: bool,
        responses: Vec<ModelResponse>,
    ) -> (RecordingMcp, SourceCalls, PolicyCalls, PortCalls) {
        let source_calls = Arc::new(Mutex::new(0));
        let policy_calls = Arc::new(Mutex::new(vec![]));
        let ports = PortCalls::default();
        let server = RecordingMcp::new(
            vec![],
            RecordingStore(ports.clone()),
            RecordingCatalog(ports.clone()),
            RecordingModel {
                calls: ports.clone(),
                responses: Mutex::new(responses),
            },
            RecordingTools(ports.clone()),
            RecordingMemory(ports.clone()),
            RecordingKnowledge(ports.clone()),
            RecordingSandbox(ports.clone()),
            AgentPolicyContextResolver::new(
                Source {
                    context: if source_ok {
                        Ok(trusted())
                    } else {
                        Err(DefinitionError::AdapterFailure)
                    },
                    calls: Arc::clone(&source_calls),
                },
                Policy {
                    allow,
                    tamper,
                    grant: None,
                    calls: Arc::clone(&policy_calls),
                },
            ),
        )
        .expect("server");
        (server, source_calls, policy_calls, ports)
    }

    fn recording_call(server: &RecordingMcp, operation: Operation) -> String {
        match operation {
            Operation::Validate => server.validate_json(definition()).expect("validate"),
            Operation::Get => tool_response(server.get_json(AgentIdInput {
                id: "agent".to_owned(),
            })),
            Operation::List => tool_response(server.list_json()),
            Operation::Register => tool_response(server.register_json(AgentDefinitionInput {
                id: "registered".to_owned(),
                ..definition()
            })),
            Operation::Invoke => tool_response(server.invoke_json(InvokeInput {
                id: "agent".to_owned(),
                input: String::new(),
            })),
        }
    }

    #[test]
    fn policy_failures_are_pre_domain_for_every_mcp_operation() {
        for (source_ok, allow, tamper, expected) in [
            (false, true, false, "operation_failed"),
            (true, false, false, "not_found"),
            (true, true, true, "operation_failed"),
        ] {
            for operation in [
                Operation::Validate,
                Operation::Get,
                Operation::List,
                Operation::Register,
                Operation::Invoke,
            ] {
                let (server, source_calls, policy_calls, ports) =
                    recording_service(source_ok, allow, tamper, vec![]);
                assert!(
                    recording_call(&server, operation).contains(expected),
                    "{expected:?} for {:?}",
                    operation.capability()
                );
                assert_eq!(*source_calls.lock().expect("source calls"), 1);
                let calls = policy_calls.lock().expect("policy calls");
                if source_ok {
                    assert_eq!(calls.as_slice(), &[operation.capability()]);
                } else {
                    assert!(calls.is_empty());
                }
                ports.assert_empty();
            }
        }
    }

    fn enabled_definition() -> AgentDefinitionV1 {
        AgentDefinitionInput {
            allowed_tool_ids: vec!["allowed-tool".to_owned(), "denied-tool".to_owned()],
            memory_enabled: true,
            memory_max_items: 1,
            knowledge_enabled: true,
            knowledge_max_results: 1,
            sandbox_allow_execution: true,
            communication_allow_messages: true,
            max_tool_calls: 2,
            max_output_bytes: 100,
            ..definition()
        }
        .into_core()
        .expect("enabled definition")
    }
    fn restricted_grant() -> GrantV1 {
        GrantV1::new(vec!["allowed-tool".to_owned()], false, false, false, false).expect("grant")
    }

    #[test]
    #[allow(clippy::too_many_lines)] // One scenario intentionally records the full MCP-to-runtime boundary.
    fn mcp_runtime_applies_policy_ceiling_to_model_and_denied_requests() {
        let source_calls = Arc::new(Mutex::new(0));
        let policy_calls = Arc::new(Mutex::new(vec![]));
        let ports = PortCalls::default();
        let responses = vec![
            ModelResponse {
                output: "ok".to_owned(),
                tool_calls: vec![agent_core::ToolCall {
                    tool_id: "allowed-tool".to_owned(),
                    input: String::new(),
                }],
                capability_requests: vec![],
            },
            ModelResponse {
                output: "ok".to_owned(),
                tool_calls: vec![agent_core::ToolCall {
                    tool_id: "denied-tool".to_owned(),
                    input: String::new(),
                }],
                capability_requests: vec![],
            },
            ModelResponse {
                output: "ok".to_owned(),
                tool_calls: vec![],
                capability_requests: vec![agent_core::CapabilityRequest::MemoryRecall {
                    query: String::new(),
                }],
            },
            ModelResponse {
                output: "ok".to_owned(),
                tool_calls: vec![],
                capability_requests: vec![agent_core::CapabilityRequest::KnowledgeSearch {
                    query: String::new(),
                }],
            },
            ModelResponse {
                output: "ok".to_owned(),
                tool_calls: vec![],
                capability_requests: vec![agent_core::CapabilityRequest::SandboxExecute {
                    action: "action".to_owned(),
                    arguments: vec![],
                }],
            },
        ];
        let server = RecordingMcp::new(
            vec![enabled_definition()],
            RecordingStore(ports.clone()),
            RecordingCatalog(ports.clone()),
            RecordingModel {
                calls: ports.clone(),
                responses: Mutex::new(responses),
            },
            RecordingTools(ports.clone()),
            RecordingMemory(ports.clone()),
            RecordingKnowledge(ports.clone()),
            RecordingSandbox(ports.clone()),
            AgentPolicyContextResolver::new(
                Source {
                    context: Ok(trusted()),
                    calls: Arc::clone(&source_calls),
                },
                Policy {
                    allow: true,
                    tamper: false,
                    grant: Some(restricted_grant()),
                    calls: Arc::clone(&policy_calls),
                },
            ),
        )
        .expect("server");
        ports.clear();
        MODEL_SCOPES.with(|scopes| scopes.lock().expect("scopes").clear());
        assert!(
            server
                .invoke_json(InvokeInput {
                    id: "agent".to_owned(),
                    input: String::new()
                })
                .expect("allowed invoke")
                .contains("allowed-tool")
        );
        assert!(ports.0.lock().expect("ports").contains(&"tools.invoke"));
        let before_denied_tool = ports.0.lock().expect("ports").len();
        assert_eq!(
            tool_response(server.invoke_json(InvokeInput {
                id: "agent".to_owned(),
                input: String::new()
            })),
            "{\"error\":\"tool_disallowed\"}"
        );
        assert_eq!(
            ports.0.lock().expect("ports").len(),
            before_denied_tool + 2,
            "only allowed-tool resolution and model invocation precede denied tool"
        );
        for expected in ["memory_denied", "knowledge_denied", "sandbox_denied"] {
            let before = ports.0.lock().expect("ports").len();
            assert_eq!(
                tool_response(server.invoke_json(InvokeInput {
                    id: "agent".to_owned(),
                    input: String::new()
                })),
                format!("{{\"error\":\"{expected}\"}}")
            );
            assert_eq!(
                ports.0.lock().expect("ports").len(),
                before + 2,
                "{expected} reaches no corresponding port"
            );
        }
        MODEL_SCOPES.with(|scopes| {
            for scope in scopes.lock().expect("scopes").iter() {
                assert_eq!(scope.allowed_tool_ids, vec!["allowed-tool"]);
                assert!(
                    !scope.memory.enabled
                        && !scope.knowledge.enabled
                        && !scope.sandbox.allow_execution
                        && !scope.communication.allow_messages
                );
            }
        });
        assert_eq!(*source_calls.lock().expect("source calls"), 5);
        assert_eq!(
            policy_calls.lock().expect("policy calls").as_slice(),
            &[CapabilityV1::AgentInvoke; 5]
        );
    }

    #[test]
    fn global_v1_definitions_are_visible_to_each_authorized_trusted_tenant() {
        for tenant in ["tenant-a", "tenant-b"] {
            let context = TrustedContextV1 {
                tenant_id: TenantId::new(tenant).expect("tenant"),
                ..trusted()
            };
            let (server, _, _) = service_with_context(context, true, true, false);
            assert!(
                server
                    .get_json(AgentIdInput {
                        id: "agent".to_owned()
                    })
                    .expect("global definition")
                    .contains("\"id\":\"agent\"")
            );
            assert!(
                server
                    .list_json()
                    .expect("global definitions")
                    .contains("\"id\":\"agent\"")
            );
        }
    }
}
