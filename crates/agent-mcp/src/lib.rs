#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)] // MCP port errors use the stable public response contract.
#![allow(clippy::needless_pass_by_value)] // Serialization consumes its transient JSON value.

//! Bounded MCP control-plane adapter for agent definitions and local invocation.

use agent_core::{
    AgentDefinitionV1, AgentId, AgentRegistry, CommunicationPolicy, DefinitionError,
    DefinitionStore, DefinitionVersion, ExecutionLimits, KnowledgePolicy, KnowledgeStore,
    LocalAgentRuntime, MemoryPolicy, MemoryStore, ModelPolicy, ModelProvider, PublicErrorCode,
    ReferenceCatalog, Sandbox, SandboxPolicy, ToolRegistry, validate_definition,
};
use anyhow::{Context, Result};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router,
    transport::stdio,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const MAX_MCP_SERIALIZED_RESULT_BYTES: usize = 65_536;

/// The complete bounded MCP tool surface.
pub const AGENT_DEFINITION_TOOLS: [&str; 5] = [
    "agent_definition_validate",
    "agent_definition_get",
    "agent_definition_list",
    "agent_definition_register",
    "agent_runtime_invoke",
];

/// JSON-friendly version-one definition input. Unknown fields are rejected.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
#[allow(clippy::struct_excessive_bools)] // Flat booleans keep the MCP input schema explicit and bounded.
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
    fn into_core(self) -> Result<AgentDefinitionV1, DefinitionError> {
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
/// Input for get and invocation operations.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct AgentIdInput {
    pub id: String,
}
/// Input for one bounded, attempt-local invocation.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct InvokeInput {
    pub id: String,
    pub input: String,
}

/// MCP server whose core ports are injected by the embedding application.
pub struct AgentDefinitionMcp<S, C, M, T, MM, K, SB>
where
    S: DefinitionStore,
    C: ReferenceCatalog,
    M: ModelProvider,
    T: ToolRegistry,
    MM: MemoryStore,
    K: KnowledgeStore,
    SB: Sandbox,
{
    registry: AgentRegistry<S, C>,
    model: M,
    tools: T,
    memory: MM,
    knowledge: K,
    sandbox: SB,
    tool_router: ToolRouter<Self>,
}
impl<S, C, M, T, MM, K, SB> AgentDefinitionMcp<S, C, M, T, MM, K, SB>
where
    S: DefinitionStore + 'static,
    C: ReferenceCatalog + 'static,
    M: ModelProvider + 'static,
    T: ToolRegistry + 'static,
    MM: MemoryStore + 'static,
    K: KnowledgeStore + 'static,
    SB: Sandbox + 'static,
{
    /// Creates an MCP server with exclusively injected core ports.
    #[allow(clippy::too_many_arguments)] // Each independently replaceable core port is injected explicitly.
    pub fn new(
        builtins: Vec<AgentDefinitionV1>,
        store: S,
        catalog: C,
        model: M,
        tools: T,
        memory: MM,
        knowledge: K,
        sandbox: SB,
    ) -> Result<Self, DefinitionError> {
        Ok(Self {
            registry: AgentRegistry::new(builtins, store, catalog)?,
            model,
            tools,
            memory,
            knowledge,
            sandbox,
            tool_router: Self::tool_router(),
        })
    }
    /// Serves MCP over standard input/output.
    pub async fn serve_stdio(self) -> Result<()> {
        self.serve(stdio()).await?.waiting().await?;
        Ok(())
    }
    fn validate_json(&self, input: AgentDefinitionInput) -> Result<String> {
        match input
            .into_core()
            .and_then(|definition| self.registry.validate(&definition))
        {
            Ok(()) => serialize(json!({"valid": true, "findings": []})),
            Err(error) => {
                serialize(json!({"valid": false, "error": public_code(error.public_code())}))
            }
        }
    }
    fn get_json(&self, input: AgentIdInput) -> Result<String> {
        let id = AgentId::new(input.id).map_err(|_| DefinitionError::InvalidDefinition)?;
        let definition = self
            .registry
            .get(&id)
            .map_err(|error| anyhow::anyhow!(public_code(error.public_code())))?;
        definition_json(&definition)
    }
    fn list_json(&self) -> Result<String> {
        let definitions = self
            .registry
            .list()
            .map_err(|error| anyhow::anyhow!(public_code(error.public_code())))?;
        serialize(
            json!({"agents": definitions.into_iter().map(|definition| json!({"id": definition.id.as_str(), "name": definition.name, "description": definition.description})).collect::<Vec<_>>() }),
        )
    }
    fn register_json(&self, input: AgentDefinitionInput) -> Result<String> {
        let definition = input
            .into_core()
            .map_err(|error| anyhow::anyhow!(public_code(error.public_code())))?;
        self.registry
            .register(definition.clone())
            .map_err(|error| anyhow::anyhow!(public_code(error.public_code())))?;
        serialize(json!({"id": definition.id.as_str(), "registered": true}))
    }
    fn invoke_json(&self, input: InvokeInput) -> Result<String> {
        let id = AgentId::new(input.id).map_err(|_| anyhow::anyhow!("invalid_definition"))?;
        let runtime = LocalAgentRuntime::new(
            &self.registry,
            &self.model,
            &self.tools,
            &self.memory,
            &self.knowledge,
            &self.sandbox,
        );
        let result = runtime
            .invoke(&id, input.input)
            .map_err(|error| anyhow::anyhow!(public_code(error.public_code())))?;
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
}

#[tool_router(router = tool_router)]
impl<S, C, M, T, MM, K, SB> AgentDefinitionMcp<S, C, M, T, MM, K, SB>
where
    S: DefinitionStore + 'static,
    C: ReferenceCatalog + 'static,
    M: ModelProvider + 'static,
    T: ToolRegistry + 'static,
    MM: MemoryStore + 'static,
    K: KnowledgeStore + 'static,
    SB: Sandbox + 'static,
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
    async fn agent_definition_list(&self) -> String {
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
impl<S, C, M, T, MM, K, SB> ServerHandler for AgentDefinitionMcp<S, C, M, T, MM, K, SB>
where
    S: DefinitionStore + 'static,
    C: ReferenceCatalog + 'static,
    M: ModelProvider + 'static,
    T: ToolRegistry + 'static,
    MM: MemoryStore + 'static,
    K: KnowledgeStore + 'static,
    SB: Sandbox + 'static,
{
}

fn definition_json(definition: &AgentDefinitionV1) -> Result<String> {
    serialize(
        json!({"version": definition.version.as_str(), "id": definition.id.as_str(), "name": definition.name, "description": definition.description, "model_reference": definition.model.reference, "instructions": definition.instructions, "skills": definition.skills, "steering": definition.steering, "allowed_tool_ids": definition.allowed_tool_ids, "memory": {"enabled": definition.memory.enabled, "max_items": definition.memory.max_items}, "knowledge": {"enabled": definition.knowledge.enabled, "max_results": definition.knowledge.max_results}, "sandbox": {"allow_execution": definition.sandbox.allow_execution}, "communication": {"allow_messages": definition.communication.allow_messages}, "limits": {"max_tool_calls": definition.limits.max_tool_calls, "max_output_bytes": definition.limits.max_output_bytes}}),
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
/// Returns MCP tool names in stable order.
#[must_use]
pub const fn tool_names() -> [&'static str; 5] {
    AGENT_DEFINITION_TOOLS
}

#[cfg(test)]
mod tests {
    use super::*;
    use agent_core::{
        DenySandbox, FixedToolRegistry, InMemoryDefinitionStore, InMemoryMemoryStore,
        ModelResponse, StaticKnowledgeStore, StaticModelProvider, StaticReferenceCatalog,
    };

    type TestMcp = AgentDefinitionMcp<
        InMemoryDefinitionStore,
        StaticReferenceCatalog,
        StaticModelProvider,
        FixedToolRegistry,
        InMemoryMemoryStore,
        StaticKnowledgeStore,
        DenySandbox,
    >;

    fn input() -> AgentDefinitionInput {
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
    fn server(response: ModelResponse) -> TestMcp {
        TestMcp::new(
            vec![input().into_core().expect("definition")],
            InMemoryDefinitionStore::default(),
            StaticReferenceCatalog::new(["static".to_owned()], [], [], []),
            StaticModelProvider::new(response),
            FixedToolRegistry::default(),
            InMemoryMemoryStore::default(),
            StaticKnowledgeStore::default(),
            DenySandbox,
        )
        .expect("server")
    }
    fn response(output: &str) -> ModelResponse {
        ModelResponse {
            output: output.to_owned(),
            tool_calls: vec![],
            capability_requests: vec![],
        }
    }

    #[test]
    fn exposes_only_specified_tools_and_schemas_reject_unknown_fields() {
        assert_eq!(
            tool_names(),
            [
                "agent_definition_validate",
                "agent_definition_get",
                "agent_definition_list",
                "agent_definition_register",
                "agent_runtime_invoke"
            ]
        );
        for schema in [
            TestMcp::agent_definition_validate_tool_attr().input_schema,
            TestMcp::agent_definition_get_tool_attr().input_schema,
            TestMcp::agent_definition_list_tool_attr().input_schema,
            TestMcp::agent_definition_register_tool_attr().input_schema,
            TestMcp::agent_runtime_invoke_tool_attr().input_schema,
        ] {
            assert!(schema.contains_key("type"));
        }
        let mut definition = serde_json::to_value(input()).expect("json");
        definition["unexpected"] = json!(true);
        assert!(serde_json::from_value::<AgentDefinitionInput>(definition).is_err());
        assert!(
            serde_json::from_value::<AgentIdInput>(json!({"id":"agent","unexpected":true}))
                .is_err()
        );
        assert!(
            serde_json::from_value::<InvokeInput>(
                json!({"id":"agent","input":"x","unexpected":true})
            )
            .is_err()
        );
    }

    #[test]
    fn complete_get_shape_includes_stable_version() {
        let result: serde_json::Value = serde_json::from_str(
            &server(response("ok"))
                .get_json(AgentIdInput {
                    id: "agent".to_owned(),
                })
                .expect("get"),
        )
        .expect("json");
        assert_eq!(result["version"], "v1");
        assert_eq!(result.as_object().expect("object").len(), 14);
        for field in [
            "id",
            "name",
            "description",
            "model_reference",
            "instructions",
            "skills",
            "steering",
            "allowed_tool_ids",
            "memory",
            "knowledge",
            "sandbox",
            "communication",
            "limits",
        ] {
            assert!(result.get(field).is_some(), "missing {field}");
        }
    }

    #[test]
    fn public_error_mappings_and_handler_responses_do_not_leak_details() {
        for code in [
            PublicErrorCode::InvalidDefinition,
            PublicErrorCode::InvalidReference,
            PublicErrorCode::ReferenceUnavailable,
            PublicErrorCode::NotFound,
            PublicErrorCode::BuiltinProtected,
            PublicErrorCode::UnknownTool,
            PublicErrorCode::ToolDisallowed,
            PublicErrorCode::MemoryDenied,
            PublicErrorCode::KnowledgeDenied,
            PublicErrorCode::SandboxDenied,
            PublicErrorCode::AdapterFailure,
            PublicErrorCode::LimitExceeded,
        ] {
            assert_eq!(
                tool_response(Err(anyhow::anyhow!(public_code(code)))),
                format!("{{\"error\":\"{}\"}}", public_code(code))
            );
        }
        for detail in ["credential=secret", "provider-token", "/private/host/path"] {
            let result = tool_response(Err(anyhow::anyhow!(detail)));
            assert_eq!(result, "{\"error\":\"operation_failed\"}");
            assert!(!result.contains(detail));
        }
    }

    #[test]
    fn final_mcp_json_framing_is_hard_bounded() {
        let result = tool_response(serialize(json!({
            "output": "x".repeat(MAX_MCP_SERIALIZED_RESULT_BYTES),
        })));
        assert_eq!(result, "{\"error\":\"limit_exceeded\"}");
    }

    #[test]
    fn invoke_has_a_stable_bounded_public_shape() {
        let result: serde_json::Value = serde_json::from_str(
            &server(response("1234567890"))
                .invoke_json(InvokeInput {
                    id: "agent".to_owned(),
                    input: String::new(),
                })
                .expect("invoke"),
        )
        .expect("json");
        assert_eq!(result["output"], "1234567890");
        assert!(
            result["capability_scope_digest"]
                .as_str()
                .is_some_and(|digest| digest.len() == 64)
        );
        assert_eq!(result["events"], json!([{"type":"model_invoked"}]));
        let bounded = tool_response(server(response("12345678901")).invoke_json(InvokeInput {
            id: "agent".to_owned(),
            input: String::new(),
        }));
        assert_eq!(bounded, "{\"error\":\"limit_exceeded\"}");
    }
}
