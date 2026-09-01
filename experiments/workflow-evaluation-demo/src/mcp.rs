//! Bounded `factory_*` MCP handler over the static demo [`Composition`].
//!
//! Tenant, principal, request, and correlation identity always come from the
//! fixed constants baked into [`Composition`] at startup. No tool input field
//! ever supplies identity, tenant, or grants.

use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    model::{Implementation, ServerCapabilities, ServerInfo},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::composition::{Composition, CompositionError};

/// The exact and only tools this server exposes, in declaration order.
pub const FACTORY_TOOLS: [&str; 4] = [
    "factory_capabilities",
    "factory_run_demo",
    "factory_get_run",
    "factory_query_telemetry",
];

#[derive(Clone, Copy, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EmptyInput {}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetRunInput {
    /// Logical run id previously returned by `factory_run_demo`.
    pub run_id: String,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct QueryTelemetryInput {
    /// Maximum number of records to return, bounded server-side.
    pub limit: usize,
}

/// The unified `factory_*` MCP handler for this project's one binary process.
pub struct FactoryMcp {
    composition: Composition,
    tool_router: ToolRouter<Self>,
}

impl FactoryMcp {
    #[must_use]
    pub fn new(composition: Composition) -> Self {
        Self {
            composition,
            tool_router: Self::tool_router(),
        }
    }

    fn capabilities_json(&self) -> String {
        serialize(json!({
            "name": "workflow-evaluation-demo",
            "version": env!("CARGO_PKG_VERSION"),
            "tools": FACTORY_TOOLS,
        }))
    }

    async fn run_demo_json(&self) -> String {
        match self.composition.run_demo().await {
            Ok(outcome) => serialize(json!({
                "run_id": outcome.run_id,
                "evidence_digest": outcome.evidence_digest,
                "result_digest": outcome.result_digest,
                "verdict": outcome.verdict,
            })),
            Err(error) => error_response(error),
        }
    }

    fn get_run_json(&self, input: &GetRunInput) -> String {
        match self.composition.get_run(&input.run_id) {
            Ok(Some(view)) => serialize(json!({
                "run_id": view.run_id,
                "status": view.status,
                "terminal_reason": view.terminal_reason,
                "output": view.output,
            })),
            Ok(None) => serialize(json!({"error": "not_found"})),
            Err(error) => error_response(error),
        }
    }

    fn query_telemetry_json(&self, input: &QueryTelemetryInput) -> String {
        match self.composition.query_telemetry(input.limit) {
            Ok(records) => serialize(json!({
                "records": records
                    .into_iter()
                    .map(|record| json!({
                        "workflow_run_id": record.workflow_run_id,
                        "evidence_digest": record.evidence_digest,
                        "result_digest": record.result_digest,
                        "verdict": record.verdict,
                    }))
                    .collect::<Vec<_>>(),
            })),
            Err(error) => error_response(error),
        }
    }
}

#[tool_router(router = tool_router)]
impl FactoryMcp {
    #[tool(
        name = "factory_capabilities",
        description = "Describe this server's name, version, and registered tools."
    )]
    async fn factory_capabilities(&self, Parameters(_input): Parameters<EmptyInput>) -> String {
        self.capabilities_json()
    }

    #[tool(
        name = "factory_run_demo",
        description = "Run one fixed-identity Agent to Workflow to Evaluation to Observability demo cycle."
    )]
    async fn factory_run_demo(&self, Parameters(_input): Parameters<EmptyInput>) -> String {
        self.run_demo_json().await
    }

    #[tool(
        name = "factory_get_run",
        description = "Get one tenant-fixed demo workflow run by id."
    )]
    async fn factory_get_run(&self, Parameters(input): Parameters<GetRunInput>) -> String {
        self.get_run_json(&input)
    }

    #[tool(
        name = "factory_query_telemetry",
        description = "Query bounded tenant-fixed demo telemetry, returning only allowlisted attributes."
    )]
    async fn factory_query_telemetry(
        &self,
        Parameters(input): Parameters<QueryTelemetryInput>,
    ) -> String {
        self.query_telemetry_json(&input)
    }
}

#[tool_handler(router = self.tool_router)]
impl ServerHandler for FactoryMcp {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_server_info(
            Implementation::new("workflow-evaluation-demo", env!("CARGO_PKG_VERSION")),
        )
    }
}

fn error_response(error: CompositionError) -> String {
    let code = match error {
        CompositionError::InvalidRequest => "invalid_request",
        CompositionError::LimitExceeded => "limit_exceeded",
        CompositionError::OperationFailed => "operation_failed",
    };
    serialize(json!({"error": code}))
}

fn serialize(value: serde_json::Value) -> String {
    serde_json::to_string(&value).unwrap_or_else(|_| r#"{"error":"operation_failed"}"#.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_declared_tool_uses_the_reserved_factory_prefix_with_no_duplicates() {
        assert_eq!(FACTORY_TOOLS.len(), 4);
        let mut seen = std::collections::BTreeSet::new();
        for name in FACTORY_TOOLS {
            assert!(
                name.starts_with("factory_"),
                "{name} missing factory_ prefix"
            );
            assert!(
                name.chars()
                    .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_'),
                "{name} violates snake_case grammar"
            );
            assert!(seen.insert(name), "duplicate tool name {name}");
        }
    }

    #[test]
    fn schemas_are_closed() {
        for schema in [
            serde_json::to_value(schemars::schema_for!(EmptyInput)).expect("empty schema"),
            serde_json::to_value(schemars::schema_for!(GetRunInput)).expect("run schema"),
            serde_json::to_value(schemars::schema_for!(QueryTelemetryInput))
                .expect("telemetry schema"),
        ] {
            assert_eq!(schema["additionalProperties"], false);
        }
    }
}
