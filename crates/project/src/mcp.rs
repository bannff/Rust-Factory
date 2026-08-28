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
#[serde(rename_all = "snake_case")]
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
}

impl<W> ProjectAuthoringMcp<W>
where
    W: ProjectWriter + Send + Sync + 'static,
    W::Error: Error + Send + Sync + 'static,
{
    /// Creates a server with an injected project writer adapter.
    #[must_use]
    pub fn new(writer: W) -> Self {
        Self {
            author: DefaultProjectAuthor,
            writer,
            tool_router: Self::tool_router(),
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
}
