//! Bounded MCP control-plane adapter for Rust projects.
//!
//! Enabled by the `mcp` feature. Owns transport DTOs, generated schemas, and
//! safe response projection — never process lifecycle.

#![allow(unknown_lints)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::unused_async_trait_impl)]

use std::error::Error;

use crate::{
    DefaultProjectAuthor, FindingSeverity, ProjectAuthor, ProjectBlueprint, ProjectKind,
    ProjectTarget, ProjectWriter, ValidationCode,
};
use anyhow::{Context, Result};
use mcp_contract::{
    DispatchError, DispatchFuture, DispatchOutcome, HandlerContribution, Namespace, ToolDescriptor,
    ToolName,
};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

/// The complete, intentionally small public tool surface for this brick.
pub const PROJECT_TOOLS: [&str; 3] = ["project_validate", "project_plan", "project_generate"];

/// JSON-friendly input for the MCP project tools.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct BlueprintInput {
    /// Human-readable workspace and destination name.
    pub workspace_name: String,
    /// Generated Cargo package name.
    pub package_name: String,
    /// Generated crate kind.
    pub kind: ProjectKindInput,
    /// SPDX license expression.
    pub license: String,
    /// Optional Cargo package description.
    pub description: Option<String>,
}

/// Supported generated crate kinds.
#[derive(Clone, Copy, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ProjectKindInput {
    /// Generate a library crate.
    Library,
    /// Generate a binary crate.
    Binary,
}

impl From<ProjectKindInput> for ProjectKind {
    fn from(value: ProjectKindInput) -> Self {
        match value {
            ProjectKindInput::Library => Self::Library,
            ProjectKindInput::Binary => Self::Binary,
        }
    }
}

impl BlueprintInput {
    fn into_core(self) -> ProjectBlueprint {
        ProjectBlueprint::v1(
            self.workspace_name,
            self.package_name,
            self.kind.into(),
            self.license,
            self.description,
        )
    }
}

/// Input for the materializing MCP operation.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct GenerateInput {
    /// Blueprint to validate, plan, and generate.
    pub blueprint: BlueprintInput,
    /// One approved directory name relative to the configured output root.
    pub target: String,
}

/// Project MCP server state.
pub struct ProjectAuthoringMcp<W>
where
    W: ProjectWriter + Send + Sync + 'static,
    W::Error: Error + Send + Sync + 'static,
{
    author: DefaultProjectAuthor,
    writer: W,
    tool_router: ToolRouter<Self>,
    namespace: Namespace,
}

impl<W> ProjectAuthoringMcp<W>
where
    W: ProjectWriter + Send + Sync + 'static,
    W::Error: Error + Send + Sync + 'static,
{
    /// Creates a server with an injected project writer adapter.
    ///
    /// # Panics
    ///
    /// Never panics in practice: the literal namespace `"project"` always
    /// satisfies [`Namespace::new`]'s closed grammar.
    #[must_use]
    pub fn new(writer: W) -> Self {
        Self {
            author: DefaultProjectAuthor,
            writer,
            tool_router: Self::tool_router(),
            namespace: Namespace::new("project").expect("literal namespace is valid"),
        }
    }

    fn validate_json(&self, input: BlueprintInput) -> Result<String> {
        let report = self.author.validate(&input.into_core());
        let findings = report
            .findings
            .into_iter()
            .map(|finding| {
                json!({
                    "code": validation_code_name(finding.code),
                    "severity": severity_name(finding.severity),
                    "field": finding.field,
                    "message": finding.message,
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&json!({
            "valid": !findings.iter().any(|finding| finding["severity"] == "error"),
            "findings": findings,
        }))
        .context("could not serialize validation response")
    }

    fn plan_json(&self, input: BlueprintInput) -> Result<String> {
        let plan = self
            .author
            .plan(&input.into_core())
            .map_err(|error| anyhow::anyhow!(error))?;
        let files = plan
            .files
            .into_iter()
            .map(|file| {
                json!({
                    "relative_path": file.relative_path().display().to_string(),
                    "contents": file.contents(),
                })
            })
            .collect::<Vec<_>>();
        serde_json::to_string(&json!({
            "files": files,
            "quality_commands": plan.quality_commands.into_iter().map(|command| json!({
                "program": command.program,
                "arguments": command.arguments,
            })).collect::<Vec<_>>(),
        }))
        .context("could not serialize plan response")
    }

    fn generate_json(&self, input: GenerateInput) -> Result<String> {
        let plan = self
            .author
            .plan(&input.blueprint.into_core())
            .map_err(|error| anyhow::anyhow!(error))?;
        let target = ProjectTarget::new(input.target).map_err(|error| anyhow::anyhow!(error))?;
        let project = self
            .writer
            .write(&plan, &target)
            .map_err(|error| anyhow::anyhow!(error))?;
        serde_json::to_string(&json!({
            "target": project.target.as_str(),
            "written_files": project.written_files.into_iter().map(|path| path.display().to_string()).collect::<Vec<_>>(),
        }))
        .context("could not serialize generation response")
    }
}

#[tool_router(router = tool_router)]
impl<W> ProjectAuthoringMcp<W>
where
    W: ProjectWriter + Send + Sync + 'static,
    W::Error: Error + Send + Sync + 'static,
{
    #[tool(
        name = "project_validate",
        description = "Validate a declarative Rust project blueprint without writing files."
    )]
    async fn project_validate(&self, Parameters(input): Parameters<BlueprintInput>) -> String {
        tool_response(self.validate_json(input))
    }

    #[tool(
        name = "project_plan",
        description = "Create a deterministic dry-run Rust workspace generation plan without writing files."
    )]
    async fn project_plan(&self, Parameters(input): Parameters<BlueprintInput>) -> String {
        tool_response(self.plan_json(input))
    }

    #[tool(
        name = "project_generate",
        description = "Safely generate a validated Rust workspace below the configured output root."
    )]
    async fn project_generate(&self, Parameters(input): Parameters<GenerateInput>) -> String {
        tool_response(self.generate_json(input))
    }
}

#[tool_handler(router = self.tool_router)]
impl<W> ServerHandler for ProjectAuthoringMcp<W>
where
    W: ProjectWriter + Send + Sync + 'static,
    W::Error: Error + Send + Sync + 'static,
{
}

fn tool_response(result: Result<String>) -> String {
    result.unwrap_or_else(|_| json!({ "error": "generation_failed" }).to_string())
}

/// The two mandatory generated discovery tools this contribution exposes.
const PROJECT_DISCOVERY_TOOLS: [&str; 2] = ["project_capabilities", "project_schema"];

/// Closed empty input for the two mandatory discovery tools.
#[derive(Clone, Debug, Deserialize, Serialize, JsonSchema)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}

/// This contribution's `HandlerContribution` migration is intentionally
/// scoped to introspection-only and non-effectful tools. `project_generate`
/// materializes files through [`ProjectWriter::write`] with no host-derived
/// trusted context or Policy authorization anywhere in this type; wiring it
/// into [`HandlerContribution::dispatch`] unchanged would expose that
/// ungated write effect on the one unified project MCP server that
/// `mcp-transport::AggregateRouter` composes. `project_generate` itself
/// remains reachable only through this brick's existing standalone
/// `ServerHandler`, with no change to its behavior or public tool contract.
/// (Its input DTOs did pick up the closed-schema `deny_unknown_fields`
/// tightening below, shared with the new discovery surface — a deliberate,
/// uniform boundary hardening, not a functional change.) This is tracked,
/// named migration debt (see GitHub issue #50), not a silent omission: it
/// requires a `TrustedContextSource`/`PolicyResolver` authorization gate —
/// mirroring `agent::mcp::AgentPolicyContextResolver` — before it can be
/// safely dispatched from a shared unified server.
impl<W> HandlerContribution for ProjectAuthoringMcp<W>
where
    W: ProjectWriter + Send + Sync + 'static,
    W::Error: Error + Send + Sync + 'static,
{
    fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    fn tools(&self) -> Vec<ToolDescriptor> {
        let namespace = &self.namespace;
        vec![
            descriptor(
                namespace,
                "validate",
                "Validate a declarative Rust project blueprint without writing files.",
                schema_map::<BlueprintInput>(),
            ),
            descriptor(
                namespace,
                "plan",
                "Create a deterministic dry-run Rust workspace generation plan without writing files.",
                schema_map::<BlueprintInput>(),
            ),
            descriptor(
                namespace,
                "capabilities",
                "Describe this contribution's namespace and non-effectful tool set.",
                schema_map::<EmptyInput>(),
            ),
            descriptor(
                namespace,
                "schema",
                "Return generated input schemas for this contribution's non-effectful tools.",
                schema_map::<EmptyInput>(),
            ),
        ]
    }

    fn dispatch(&self, tool: ToolName, arguments: serde_json::Value) -> DispatchFuture<'_> {
        Box::pin(async move {
            match tool.as_str() {
                "project_validate" => {
                    let input = deserialize_arguments(arguments)?;
                    Ok(fold_outcome(self.validate_json(input)))
                }
                "project_plan" => {
                    let input = deserialize_arguments(arguments)?;
                    Ok(fold_outcome(self.plan_json(input)))
                }
                "project_capabilities" => Ok(fold_outcome(Ok(self.capabilities_json()))),
                "project_schema" => Ok(fold_outcome(Ok(schema_json()))),
                _ => Err(DispatchError::UnknownTool),
            }
        })
    }
}

impl<W> ProjectAuthoringMcp<W>
where
    W: ProjectWriter + Send + Sync + 'static,
    W::Error: Error + Send + Sync + 'static,
{
    fn capabilities_json(&self) -> String {
        let mut tools = PROJECT_TOOLS
            .iter()
            .filter(|&&name| name != "project_generate")
            .copied()
            .collect::<Vec<_>>();
        tools.extend(PROJECT_DISCOVERY_TOOLS);
        json!({
            "namespace": self.namespace.as_str(),
            "tools": tools,
        })
        .to_string()
    }
}

fn schema_map<T: JsonSchema>() -> serde_json::Map<String, serde_json::Value> {
    serde_json::to_value(schemars::schema_for!(T))
        .ok()
        .and_then(|value| value.as_object().cloned())
        .unwrap_or_default()
}

fn descriptor(
    namespace: &Namespace,
    operation: &str,
    description: &str,
    schema: serde_json::Map<String, serde_json::Value>,
) -> ToolDescriptor {
    ToolDescriptor {
        name: ToolName::new(namespace, operation).expect("operation is a valid closed segment"),
        title: None,
        description: description.to_owned(),
        input_schema: schema,
        output_schema: None,
    }
}

fn deserialize_arguments<T: serde::de::DeserializeOwned>(
    arguments: serde_json::Value,
) -> std::result::Result<T, DispatchError> {
    serde_json::from_value(arguments)
        .map_err(|_| DispatchError::MalformedArguments("could not parse tool arguments".to_owned()))
}

fn fold_outcome(result: Result<String>) -> DispatchOutcome {
    let projected = tool_response(result);
    DispatchOutcome {
        payload: serde_json::from_str(&projected)
            .unwrap_or_else(|_| json!({ "error": "operation_failed" })),
        is_error: false,
    }
}

fn schema_json() -> String {
    json!({
        "project_validate": schemars::schema_for!(BlueprintInput),
        "project_plan": schemars::schema_for!(BlueprintInput),
    })
    .to_string()
}

fn validation_code_name(code: ValidationCode) -> &'static str {
    match code {
        ValidationCode::EmptyField => "empty_field",
        ValidationCode::InvalidName => "invalid_name",
        ValidationCode::EmptyDescription => "empty_description",
    }
}

fn severity_name(severity: FindingSeverity) -> &'static str {
    match severity {
        FindingSeverity::Error => "error",
        FindingSeverity::Warning => "warning",
    }
}

/// Returns the configured MCP tool names in stable order.
#[must_use]
pub const fn tool_names() -> [&'static str; 3] {
    PROJECT_TOOLS
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{GenerationPlan, MaterializedProject};

    /// A writer that records nothing and touches no filesystem.
    ///
    /// These tests exercise DTO conversion, validation projection, and generated
    /// schemas, none of which materialize a plan. Using a stub rather than the
    /// `fs` adapter keeps this module independent of the `fs` feature, so
    /// `--features mcp` alone is a valid build.
    #[derive(Clone, Copy, Debug, Default)]
    struct StubWriter;

    #[derive(Debug)]
    struct StubError;
    impl std::fmt::Display for StubError {
        fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("stub writer never writes")
        }
    }
    impl Error for StubError {}

    impl ProjectWriter for StubWriter {
        type Error = StubError;

        fn write(
            &self,
            _plan: &GenerationPlan,
            _target: &ProjectTarget,
        ) -> std::result::Result<MaterializedProject, Self::Error> {
            Err(StubError)
        }
    }

    fn input() -> BlueprintInput {
        BlueprintInput {
            workspace_name: "example".to_owned(),
            package_name: "example_crate".to_owned(),
            kind: ProjectKindInput::Library,
            license: "Apache-2.0".to_owned(),
            description: None,
        }
    }

    #[test]
    fn exposes_only_the_bounded_project_tools() {
        assert_eq!(
            tool_names(),
            ["project_validate", "project_plan", "project_generate"]
        );
    }

    #[test]
    fn validation_response_preserves_a_stable_error_code() {
        let server = ProjectAuthoringMcp::new(StubWriter);
        let response = server
            .validate_json(BlueprintInput {
                workspace_name: "bad name".to_owned(),
                ..input()
            })
            .expect("validation should return a response");
        assert!(response.contains("invalid_name"));
    }

    #[test]
    fn project_generate_schema_requires_a_target() {
        let attributes = ProjectAuthoringMcp::<StubWriter>::project_generate_tool_attr();
        assert!(attributes.input_schema["properties"]["target"].is_object());
        assert!(
            attributes.input_schema["required"]
                .as_array()
                .is_some_and(|required| required.iter().any(|value| value == "target"))
        );
    }

    fn poll_immediate<F: std::future::Future>(future: F) -> F::Output {
        let mut future = std::pin::pin!(future);
        match future
            .as_mut()
            .poll(&mut std::task::Context::from_waker(std::task::Waker::noop()))
        {
            std::task::Poll::Ready(output) => output,
            std::task::Poll::Pending => panic!("dispatch future unexpectedly pending"),
        }
    }

    #[test]
    fn handler_contribution_namespace_is_project() {
        let server = ProjectAuthoringMcp::new(StubWriter);
        assert_eq!(server.namespace().as_str(), "project");
    }

    #[test]
    fn handler_contribution_tools_exclude_project_generate_and_include_discovery_tools() {
        let server = ProjectAuthoringMcp::new(StubWriter);
        let tools = server.tools();
        let names: Vec<&str> = tools.iter().map(|tool| tool.name.as_str()).collect();
        assert_eq!(
            names,
            [
                "project_validate",
                "project_plan",
                "project_capabilities",
                "project_schema"
            ]
        );
        assert!(!names.contains(&"project_generate"));
    }

    #[test]
    fn handler_contribution_dispatch_validate_matches_the_standalone_tool_response() {
        let server = ProjectAuthoringMcp::new(StubWriter);
        let arguments = serde_json::to_value(BlueprintInput {
            workspace_name: "bad name".to_owned(),
            ..input()
        })
        .expect("serialize input");
        let tool = ToolName::new(&server.namespace, "validate").expect("tool name");
        let outcome = poll_immediate(server.dispatch(tool, arguments)).expect("dispatch succeeds");
        assert!(!outcome.is_error);
        assert!(outcome.payload.to_string().contains("invalid_name"));
    }

    /// `deny_unknown_fields` is a `serde` attribute on the DTO itself, so it
    /// applies identically regardless of which caller deserializes into it:
    /// `rmcp`'s `Parameters<T>` extractor on the standalone `ServerHandler`
    /// path, and `HandlerContribution::dispatch`'s `deserialize_arguments` on
    /// the unified-server path, both call the same generated
    /// `Deserialize::deserialize`. This test proves the DTO itself rejects an
    /// unknown field directly, independent of which caller invokes it.
    #[test]
    fn blueprint_input_rejects_unknown_fields_regardless_of_caller() {
        let mut value = serde_json::to_value(input()).expect("serialize input");
        value["unexpected_field"] = serde_json::json!("value");
        let error = serde_json::from_value::<BlueprintInput>(value)
            .expect_err("unknown field must be rejected by the DTO itself");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn generate_input_rejects_unknown_fields_regardless_of_caller() {
        let mut value = serde_json::to_value(GenerateInput {
            blueprint: input(),
            target: "example".to_owned(),
        })
        .expect("serialize input");
        value["unexpected_field"] = serde_json::json!("value");
        let error = serde_json::from_value::<GenerateInput>(value)
            .expect_err("unknown field must be rejected by the DTO itself");
        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn handler_contribution_dispatch_rejects_unknown_fields() {
        let server = ProjectAuthoringMcp::new(StubWriter);
        let mut arguments = serde_json::to_value(input()).expect("serialize input");
        arguments["unexpected_field"] = serde_json::json!("value");
        let tool = ToolName::new(&server.namespace, "validate").expect("tool name");
        let error = poll_immediate(server.dispatch(tool, arguments))
            .expect_err("unknown field must be rejected");
        assert!(matches!(error, DispatchError::MalformedArguments(_)));
    }

    #[test]
    fn handler_contribution_dispatch_rejects_project_generate_and_unknown_tools() {
        let server = ProjectAuthoringMcp::new(StubWriter);
        for operation in ["generate", "missing"] {
            let tool = ToolName::new(&server.namespace, operation).expect("tool name");
            let error = poll_immediate(server.dispatch(tool, serde_json::json!({})))
                .expect_err("must be rejected");
            assert_eq!(error, DispatchError::UnknownTool);
        }
    }

    /// `fold_outcome` never surfaces the underlying `anyhow` error text: any
    /// failure inside `validate_json`/`plan_json` is replaced with the fixed
    /// `{"error":"generation_failed"}` literal (via the pre-existing
    /// `tool_response` helper) before this contribution ever sees it, so no
    /// internal cause, path, or secret can leak through `dispatch`.
    #[test]
    fn handler_contribution_dispatch_plan_error_never_leaks_internal_detail() {
        let server = ProjectAuthoringMcp::new(StubWriter);
        let arguments = serde_json::to_value(BlueprintInput {
            package_name: String::new(),
            ..input()
        })
        .expect("serialize input");
        let tool = ToolName::new(&server.namespace, "plan").expect("tool name");
        let outcome = poll_immediate(server.dispatch(tool, arguments)).expect("dispatch succeeds");
        assert!(!outcome.is_error);
        assert_eq!(
            outcome.payload,
            serde_json::json!({"error": "generation_failed"})
        );
    }

    #[test]
    fn handler_contribution_capabilities_and_schema_are_bounded_and_safe() {
        let server = ProjectAuthoringMcp::new(StubWriter);
        let capabilities_tool = ToolName::new(&server.namespace, "capabilities").expect("tool");
        let capabilities =
            poll_immediate(server.dispatch(capabilities_tool, serde_json::json!({})))
                .expect("capabilities dispatch succeeds");
        assert!(!capabilities.is_error);
        assert_eq!(capabilities.payload["namespace"], "project");
        assert!(
            !capabilities.payload["tools"]
                .to_string()
                .contains("generate")
        );

        let schema_tool = ToolName::new(&server.namespace, "schema").expect("tool");
        let schema = poll_immediate(server.dispatch(schema_tool, serde_json::json!({})))
            .expect("schema dispatch succeeds");
        assert!(!schema.is_error);
        assert!(schema.payload["project_validate"].is_object());
    }
}
