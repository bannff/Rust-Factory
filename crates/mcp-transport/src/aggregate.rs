//! Statically composes brick [`HandlerContribution`]s and project meta-tools
//! into exactly one `rmcp::ServerHandler`.
//!
//! [`AggregateRouterBuilder`] validates namespaces, rejects duplicate tool
//! names, and enforces aggregate size budgets before returning
//! [`AggregateRouter`]; construction fails closed rather than returning a
//! partial or degraded router. [`AggregateRouter`] deliberately does not use
//! rmcp's `#[tool_router]`/`#[tool_handler]` macros: those macros generate a
//! `ToolRouter<Self>` monomorphized per concrete handler type, which is
//! exactly what prevents composing heterogeneous brick handlers into one
//! server. Instead, `AggregateRouter` implements
//! [`rmcp::handler::server::ServerHandler`] by hand, dispatching each call by
//! looking up the contribution that owns the requested tool name.
//!
//! This module performs no authorization. Each [`HandlerContribution`] binds
//! its own host-derived trusted context and Policy resolver at construction
//! time and remains solely responsible for exact capability authorization
//! immediately before any non-introspection effect or tenant-scoped read.

use std::collections::HashMap;
use std::sync::Arc;

use mcp_contract::{
    ContractError, DispatchError, DispatchOutcome, HandlerContribution, Namespace,
    ProjectMetaContribution, ToolName,
};
use rmcp::ErrorData;
use rmcp::model::{CallToolRequestParams, CallToolResponse, CallToolResult, ContentBlock, Tool};
use rmcp::service::RequestContext;
use rmcp::{RoleServer, ServerHandler};

/// Maximum brick and project-meta contributions combined.
pub const MAX_CONTRIBUTIONS: usize = 32;
/// Maximum tools across every registered contribution combined.
pub const MAX_TOOLS: usize = 256;
/// Maximum aggregate serialized bytes of every registered tool's name,
/// description, and input/output schema combined.
///
/// This is headroom against [`crate::MAX_MCP_STDIO_FRAME_BYTES`] for the
/// `tools/list` discovery payload before JSON-RPC envelope and transport
/// framing overhead, not proof that the complete wire envelope fits. The
/// transport's own `send` path independently measures and bounds the
/// complete `TxJsonRpcMessage` before every write.
pub const MAX_AGGREGATE_SCHEMA_BYTES: usize = 32 * 1024;

/// A failure to construct an [`AggregateRouter`].
///
/// Construction fails closed: no partial or degraded router is ever
/// returned.
#[derive(Debug)]
pub enum AggregateBuildError {
    InvalidNamespace(ContractError),
    /// A brick contribution declared the reserved `factory` namespace.
    ReservedNamespace(String),
    /// A project-meta contribution did not declare the reserved `factory`
    /// namespace.
    NonReservedProjectMetaNamespace(String),
    DuplicateNamespace(String),
    DuplicateToolName(String),
    /// A tool's declared name does not start with `<namespace>_`.
    ToolNameNamespaceMismatch {
        tool: String,
        namespace: String,
    },
    TooManyContributions {
        found: usize,
        max: usize,
    },
    TooManyTools {
        found: usize,
        max: usize,
    },
    SchemaBudgetExceeded {
        bytes: usize,
        max: usize,
    },
}

impl std::fmt::Display for AggregateBuildError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidNamespace(error) => write!(formatter, "invalid namespace: {error}"),
            Self::ReservedNamespace(namespace) => {
                write!(formatter, "brick namespace {namespace:?} is reserved")
            }
            Self::NonReservedProjectMetaNamespace(namespace) => write!(
                formatter,
                "project meta-tool namespace {namespace:?} must be the reserved factory namespace"
            ),
            Self::DuplicateNamespace(namespace) => {
                write!(formatter, "duplicate namespace {namespace:?}")
            }
            Self::DuplicateToolName(name) => write!(formatter, "duplicate tool name {name:?}"),
            Self::ToolNameNamespaceMismatch { tool, namespace } => write!(
                formatter,
                "tool {tool:?} does not start with namespace {namespace:?}"
            ),
            Self::TooManyContributions { found, max } => {
                write!(formatter, "{found} contributions exceeds maximum {max}")
            }
            Self::TooManyTools { found, max } => {
                write!(formatter, "{found} tools exceeds maximum {max}")
            }
            Self::SchemaBudgetExceeded { bytes, max } => write!(
                formatter,
                "aggregate schema size {bytes} bytes exceeds headroom budget {max} bytes"
            ),
        }
    }
}

impl std::error::Error for AggregateBuildError {}

struct RegisteredTool {
    tool: Tool,
    contribution_index: usize,
}

/// Builds an [`AggregateRouter`] from statically selected contributions.
#[derive(Default)]
pub struct AggregateRouterBuilder {
    bricks: Vec<Box<dyn HandlerContribution>>,
    meta: Vec<Box<dyn ProjectMetaContribution>>,
}

impl AggregateRouterBuilder {
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub fn with_brick(mut self, contribution: Box<dyn HandlerContribution>) -> Self {
        self.bricks.push(contribution);
        self
    }

    #[must_use]
    pub fn with_project_meta(mut self, contribution: Box<dyn ProjectMetaContribution>) -> Self {
        self.meta.push(contribution);
        self
    }

    /// Validates every namespace and tool name, enforces aggregate budgets,
    /// and builds one [`AggregateRouter`]. Fails closed: no partial or
    /// degraded router is ever returned.
    pub fn build(self) -> Result<AggregateRouter, AggregateBuildError> {
        let total_contributions = self.bricks.len() + self.meta.len();
        if total_contributions > MAX_CONTRIBUTIONS {
            return Err(AggregateBuildError::TooManyContributions {
                found: total_contributions,
                max: MAX_CONTRIBUTIONS,
            });
        }

        let mut contributions: Vec<Box<dyn HandlerContribution>> = Vec::new();
        let mut seen_namespaces: std::collections::HashSet<String> =
            std::collections::HashSet::new();
        let mut owners: HashMap<String, RegisteredTool> = HashMap::new();
        let mut schema_bytes = 0_usize;

        for contribution in self.bricks {
            let namespace = contribution.namespace().clone();
            if namespace.is_factory_reserved() {
                return Err(AggregateBuildError::ReservedNamespace(
                    namespace.as_str().to_owned(),
                ));
            }
            register_contribution(
                &mut contributions,
                &mut seen_namespaces,
                &mut owners,
                &mut schema_bytes,
                &namespace,
                contribution,
            )?;
        }
        for contribution in self.meta {
            let namespace = contribution.namespace().clone();
            if !namespace.is_factory_reserved() {
                return Err(AggregateBuildError::NonReservedProjectMetaNamespace(
                    namespace.as_str().to_owned(),
                ));
            }
            register_contribution(
                &mut contributions,
                &mut seen_namespaces,
                &mut owners,
                &mut schema_bytes,
                &namespace,
                contribution,
            )?;
        }

        if owners.len() > MAX_TOOLS {
            return Err(AggregateBuildError::TooManyTools {
                found: owners.len(),
                max: MAX_TOOLS,
            });
        }
        if schema_bytes > MAX_AGGREGATE_SCHEMA_BYTES {
            return Err(AggregateBuildError::SchemaBudgetExceeded {
                bytes: schema_bytes,
                max: MAX_AGGREGATE_SCHEMA_BYTES,
            });
        }

        let mut tools_cache: Vec<Tool> = owners.values().map(|entry| entry.tool.clone()).collect();
        tools_cache.sort_by(|left, right| left.name.cmp(&right.name));
        let tool_owner_by_name = owners
            .into_iter()
            .map(|(name, entry)| (name, entry.contribution_index))
            .collect();

        Ok(AggregateRouter {
            tools_cache,
            tool_owner_by_name,
            contributions,
        })
    }
}

fn register_contribution(
    contributions: &mut Vec<Box<dyn HandlerContribution>>,
    seen_namespaces: &mut std::collections::HashSet<String>,
    owners: &mut HashMap<String, RegisteredTool>,
    schema_bytes: &mut usize,
    namespace: &Namespace,
    contribution: Box<dyn HandlerContribution>,
) -> Result<(), AggregateBuildError> {
    if !seen_namespaces.insert(namespace.as_str().to_owned()) {
        return Err(AggregateBuildError::DuplicateNamespace(
            namespace.as_str().to_owned(),
        ));
    }
    let contribution_index = contributions.len();
    let prefix = format!("{namespace}_");
    for descriptor in contribution.tools() {
        let name = descriptor.name.as_str().to_owned();
        if !name.starts_with(&prefix) {
            return Err(AggregateBuildError::ToolNameNamespaceMismatch {
                tool: name,
                namespace: namespace.as_str().to_owned(),
            });
        }
        *schema_bytes += name.len()
            + descriptor.description.len()
            + descriptor.title.as_ref().map_or(0, String::len)
            + serde_json::to_string(&descriptor.input_schema)
                .map(|value| value.len())
                .unwrap_or_default()
            + descriptor
                .output_schema
                .as_ref()
                .and_then(|schema| serde_json::to_string(schema).ok())
                .map_or(0, |value| value.len());
        let mut tool = Tool::new_with_raw(
            name.clone(),
            Some(std::borrow::Cow::Owned(descriptor.description)),
            Arc::new(descriptor.input_schema),
        );
        if let Some(title) = descriptor.title {
            tool = tool.with_title(title);
        }
        if let Some(output_schema) = descriptor.output_schema {
            tool = tool.with_raw_output_schema(Arc::new(output_schema));
        }
        if owners
            .insert(
                name.clone(),
                RegisteredTool {
                    tool,
                    contribution_index,
                },
            )
            .is_some()
        {
            return Err(AggregateBuildError::DuplicateToolName(name));
        }
    }
    contributions.push(contribution);
    Ok(())
}

/// The one concrete `rmcp::ServerHandler` a project constructs.
///
/// Built only through [`AggregateRouterBuilder::build`], so every instance
/// already satisfies namespace, duplicate-name, and aggregate budget
/// validation.
pub struct AggregateRouter {
    tools_cache: Vec<Tool>,
    tool_owner_by_name: HashMap<String, usize>,
    contributions: Vec<Box<dyn HandlerContribution>>,
}

impl AggregateRouter {
    /// Returns the exact, ordered set of tool names this router serves.
    #[must_use]
    pub fn tool_names(&self) -> Vec<&str> {
        self.tools_cache
            .iter()
            .map(|tool| tool.name.as_ref())
            .collect()
    }
}

impl ServerHandler for AggregateRouter {
    async fn list_tools(
        &self,
        _request: Option<rmcp::model::PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<rmcp::model::ListToolsResult, ErrorData> {
        Ok(rmcp::model::ListToolsResult::with_all_items(
            self.tools_cache.clone(),
        ))
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, ErrorData> {
        let name = request.name.to_string();
        let Some(&contribution_index) = self.tool_owner_by_name.get(&name) else {
            return Err(ErrorData::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                format!("unknown tool {name:?}"),
                None,
            ));
        };
        let tool_name = ToolName::new(&namespace_of(&name), operation_of(&name))
            .map_err(|_| ErrorData::internal_error("malformed registered tool name", None))?;
        let arguments = request.arguments.map_or(
            serde_json::Value::Object(serde_json::Map::new()),
            serde_json::Value::Object,
        );
        let contribution = &self.contributions[contribution_index];
        match contribution.dispatch(tool_name, arguments).await {
            Ok(DispatchOutcome { payload, is_error }) => {
                let content = vec![ContentBlock::text(payload.to_string())];
                Ok(CallToolResponse::Complete(if is_error {
                    CallToolResult::error(content)
                } else {
                    CallToolResult::success(content)
                }))
            }
            Err(DispatchError::UnknownTool) => Err(ErrorData::new(
                rmcp::model::ErrorCode::METHOD_NOT_FOUND,
                format!("unknown tool {name:?}"),
                None,
            )),
            Err(DispatchError::MalformedArguments(message)) => {
                Err(ErrorData::invalid_params(message, None))
            }
            Err(DispatchError::Internal) => {
                Err(ErrorData::internal_error("internal dispatch failure", None))
            }
        }
    }
}

/// Recovers the namespace segment of an already-validated `<namespace>_<op>`
/// tool name. Only ever called on names this router itself registered.
fn namespace_of(tool_name: &str) -> Namespace {
    let namespace = tool_name
        .split_once('_')
        .map_or(tool_name, |(head, _)| head);
    Namespace::new(namespace).unwrap_or_else(|_| Namespace::factory_reserved())
}

/// Recovers the operation segment of an already-validated `<namespace>_<op>`
/// tool name. Only ever called on names this router itself registered.
fn operation_of(tool_name: &str) -> &str {
    tool_name
        .split_once('_')
        .map_or(tool_name, |(_, tail)| tail)
}

#[cfg(test)]
mod tests {
    use super::*;
    use mcp_contract::{DispatchFuture, ToolDescriptor};
    use serde_json::{Map, Value};

    struct Fixture {
        namespace: Namespace,
        tools: Vec<ToolDescriptor>,
    }
    impl HandlerContribution for Fixture {
        fn namespace(&self) -> &Namespace {
            &self.namespace
        }
        fn tools(&self) -> Vec<ToolDescriptor> {
            self.tools.clone()
        }
        fn dispatch(&self, tool: ToolName, arguments: Value) -> DispatchFuture<'_> {
            Box::pin(async move {
                Ok(mcp_contract::DispatchOutcome {
                    payload: serde_json::json!({"tool": tool.as_str(), "echo": arguments}),
                    is_error: false,
                })
            })
        }
    }
    impl ProjectMetaContribution for Fixture {}

    fn descriptor(namespace: &Namespace, operation: &str) -> ToolDescriptor {
        ToolDescriptor {
            name: ToolName::new(namespace, operation).expect("tool name"),
            title: None,
            description: format!("{operation} tool"),
            input_schema: Map::new(),
            output_schema: None,
        }
    }

    #[test]
    fn builds_and_lists_exactly_the_registered_tools_in_deterministic_order() {
        let agent_namespace = Namespace::new("agent").expect("namespace");
        let agent = Fixture {
            namespace: agent_namespace.clone(),
            tools: vec![
                descriptor(&agent_namespace, "capabilities"),
                descriptor(&agent_namespace, "schema"),
            ],
        };
        let factory_namespace = Namespace::factory_reserved();
        let factory = Fixture {
            namespace: factory_namespace.clone(),
            tools: vec![descriptor(&factory_namespace, "run_demo")],
        };

        let router = AggregateRouterBuilder::new()
            .with_brick(Box::new(agent))
            .with_project_meta(Box::new(factory))
            .build()
            .expect("router builds");
        assert_eq!(
            router.tool_names(),
            ["agent_capabilities", "agent_schema", "factory_run_demo"]
        );
    }

    #[test]
    fn rejects_reserved_namespace_from_a_brick_contribution() {
        let factory_namespace = Namespace::factory_reserved();
        let brick = Fixture {
            namespace: factory_namespace.clone(),
            tools: vec![descriptor(&factory_namespace, "sneaky")],
        };
        let Err(error) = AggregateRouterBuilder::new()
            .with_brick(Box::new(brick))
            .build()
        else {
            panic!("reserved namespace must be rejected");
        };
        assert!(matches!(error, AggregateBuildError::ReservedNamespace(_)));
    }

    #[test]
    fn rejects_non_factory_namespace_from_a_project_meta_contribution() {
        let agent_namespace = Namespace::new("agent").expect("namespace");
        let meta = Fixture {
            namespace: agent_namespace.clone(),
            tools: vec![descriptor(&agent_namespace, "sneaky")],
        };
        let Err(error) = AggregateRouterBuilder::new()
            .with_project_meta(Box::new(meta))
            .build()
        else {
            panic!("non-factory project meta namespace must be rejected");
        };
        assert!(matches!(
            error,
            AggregateBuildError::NonReservedProjectMetaNamespace(_)
        ));
    }

    #[test]
    fn rejects_duplicate_tool_names_across_two_contributions() {
        let agent_namespace = Namespace::new("agent").expect("namespace");
        let first = Fixture {
            namespace: agent_namespace.clone(),
            tools: vec![descriptor(&agent_namespace, "capabilities")],
        };
        let second = Fixture {
            namespace: agent_namespace.clone(),
            tools: vec![descriptor(&agent_namespace, "capabilities")],
        };
        let Err(error) = AggregateRouterBuilder::new()
            .with_brick(Box::new(first))
            .with_brick(Box::new(second))
            .build()
        else {
            panic!("duplicate namespace must be rejected");
        };
        assert!(matches!(error, AggregateBuildError::DuplicateNamespace(_)));
    }

    #[test]
    fn rejects_a_tool_name_that_does_not_match_its_contributions_namespace() {
        let agent_namespace = Namespace::new("agent").expect("namespace");
        let other_namespace = Namespace::new("workflow").expect("namespace");
        let mismatched = Fixture {
            namespace: agent_namespace,
            tools: vec![descriptor(&other_namespace, "capabilities")],
        };
        let Err(error) = AggregateRouterBuilder::new()
            .with_brick(Box::new(mismatched))
            .build()
        else {
            panic!("namespace mismatch must be rejected");
        };
        assert!(matches!(
            error,
            AggregateBuildError::ToolNameNamespaceMismatch { .. }
        ));
    }

    /// Regression guard: a naive implementation could catch every duplicate
    /// tool name as a `DuplicateNamespace` because same-namespace duplicates
    /// always collide on namespace first. This constructs two contributions
    /// with genuinely *different* namespaces (`foo` and `foo_bar`) whose
    /// joined tool names still collide as raw strings
    /// (`foo` + `bar_baz` == `foo_bar` + `baz` == `"foo_bar_baz"`), proving
    /// the `DuplicateToolName` branch is reachable and enforced independent
    /// of namespace collision.
    #[test]
    fn rejects_duplicate_tool_names_across_two_different_namespaces() {
        let foo_namespace = Namespace::new("foo").expect("namespace");
        let foo_bar_namespace = Namespace::new("foo_bar").expect("namespace");
        let first = Fixture {
            namespace: foo_namespace.clone(),
            tools: vec![descriptor(&foo_namespace, "bar_baz")],
        };
        let second = Fixture {
            namespace: foo_bar_namespace.clone(),
            tools: vec![descriptor(&foo_bar_namespace, "baz")],
        };
        assert_eq!(
            first.tools()[0].name.as_str(),
            second.tools()[0].name.as_str(),
            "fixture must actually produce colliding joined tool names"
        );
        let Err(error) = AggregateRouterBuilder::new()
            .with_brick(Box::new(first))
            .with_brick(Box::new(second))
            .build()
        else {
            panic!("cross-namespace duplicate tool name must be rejected");
        };
        assert!(
            matches!(error, AggregateBuildError::DuplicateToolName(ref name) if name == "foo_bar_baz"),
            "expected DuplicateToolName(\"foo_bar_baz\"), got {error:?}"
        );
    }

    /// Boundary (not over-limit): exactly `MAX_CONTRIBUTIONS` contributions,
    /// each declaring exactly enough tools that the aggregate total lands at
    /// exactly `MAX_TOOLS`, must build successfully.
    #[test]
    fn builds_successfully_at_exactly_the_max_contributions_and_max_tools_boundary() {
        let tools_per_contribution = MAX_TOOLS / MAX_CONTRIBUTIONS;
        assert!(
            tools_per_contribution > 0,
            "MAX_TOOLS must be large enough to divide across MAX_CONTRIBUTIONS"
        );
        let mut builder = AggregateRouterBuilder::new();
        let mut expected_tool_count = 0_usize;
        for index in 0..MAX_CONTRIBUTIONS {
            let namespace = Namespace::new(format!("brick{index}")).expect("namespace");
            let tools: Vec<ToolDescriptor> = (0..tools_per_contribution)
                .map(|tool_index| descriptor(&namespace, &format!("op{tool_index}")))
                .collect();
            expected_tool_count += tools.len();
            builder = builder.with_brick(Box::new(Fixture { namespace, tools }));
        }
        assert!(
            expected_tool_count <= MAX_TOOLS,
            "boundary fixture must stay within MAX_TOOLS"
        );
        let router = builder
            .build()
            .expect("exactly MAX_CONTRIBUTIONS contributions within MAX_TOOLS must build");
        assert_eq!(router.tool_names().len(), expected_tool_count);
    }

    /// `AggregateRouterBuilder::build()` with zero contributions is treated
    /// as valid by this implementation: it succeeds and yields an empty tool
    /// list rather than erroring. This is asserted here as the documented
    /// current behavior; whether an aggregate with no tools at all should
    /// instead be a construction error is a product decision this test
    /// intentionally does not make (see QA findings).
    #[test]
    fn build_with_zero_contributions_succeeds_with_empty_tool_list() {
        let router = AggregateRouterBuilder::new()
            .build()
            .expect("zero contributions currently builds successfully");
        assert!(router.tool_names().is_empty());
    }

    #[test]
    fn rejects_too_many_contributions() {
        let mut builder = AggregateRouterBuilder::new();
        for index in 0..=MAX_CONTRIBUTIONS {
            let namespace = Namespace::new(format!("brick{index}")).expect("namespace");
            builder = builder.with_brick(Box::new(Fixture {
                namespace: namespace.clone(),
                tools: vec![descriptor(&namespace, "capabilities")],
            }));
        }
        let Err(error) = builder.build() else {
            panic!("too many contributions");
        };
        assert!(matches!(
            error,
            AggregateBuildError::TooManyContributions { .. }
        ));
    }

    // `call_tool`/`list_tools` dispatch behavior against a real peer is
    // covered by the end-to-end transport integration test in
    // `tests/aggregate_router_smoke.rs`: constructing a standalone
    // `RequestContext<RoleServer>` here would require a real `Peer<RoleServer>`,
    // which only a live `rmcp` service session provides.
}
