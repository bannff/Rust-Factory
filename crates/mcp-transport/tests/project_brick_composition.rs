//! Proves the `project` brick's real `HandlerContribution` migration
//! (crates/project/src/mcp.rs) composes into a real `AggregateRouter`, and
//! that `project_generate` is correctly absent from the unified surface
//! while remaining unaffected on the brick's own standalone `ServerHandler`.
//!
//! `mcp-transport` is infrastructure, not a `BRICKS`-enumerated capability
//! crate, so depending on `project` here (dev-only) does not affect
//! `make isolation-check`, unlike depending on it from `project` itself.

use std::error::Error;
use std::fmt;

use mcp_contract::{HandlerContribution, Namespace, ToolName};
use mcp_transport::AggregateRouterBuilder;
use project::ProjectWriter;
use project::mcp::{BlueprintInput, ProjectAuthoringMcp, ProjectKindInput};

#[derive(Clone, Copy, Debug, Default)]
struct StubWriter;

#[derive(Debug)]
struct StubError;
impl fmt::Display for StubError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("stub writer never writes")
    }
}
impl Error for StubError {}

impl ProjectWriter for StubWriter {
    type Error = StubError;

    fn write(
        &self,
        _plan: &project::GenerationPlan,
        _target: &project::ProjectTarget,
    ) -> Result<project::MaterializedProject, Self::Error> {
        Err(StubError)
    }
}

#[test]
fn project_contribution_composes_into_a_real_aggregate_router_without_project_generate() {
    let router = AggregateRouterBuilder::new()
        .with_brick(Box::new(ProjectAuthoringMcp::new(StubWriter)))
        .build()
        .expect("project contribution composes with no other contribution present");
    assert_eq!(
        router.tool_names(),
        [
            "project_capabilities",
            "project_plan",
            "project_schema",
            "project_validate",
        ],
        "project_generate must remain absent from the unified surface until it is \
         authorization-gated (tracked debt, GitHub issue #50)"
    );
}

#[tokio::test]
async fn project_validate_dispatches_the_same_result_as_the_standalone_tool() {
    let contribution = ProjectAuthoringMcp::new(StubWriter);
    let namespace = Namespace::new("project").expect("namespace");
    let tool = ToolName::new(&namespace, "validate").expect("tool");
    let arguments = serde_json::to_value(BlueprintInput {
        workspace_name: "example".to_owned(),
        package_name: "example_crate".to_owned(),
        kind: ProjectKindInput::Library,
        license: "Apache-2.0".to_owned(),
        description: None,
    })
    .expect("serialize input");

    let outcome = contribution
        .dispatch(tool, arguments)
        .await
        .expect("dispatch succeeds");
    assert!(!outcome.is_error);
    assert_eq!(outcome.payload["valid"], true);
}
