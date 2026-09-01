#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)] // Constructors share the typed ContractError contract.

//! Transport-agnostic shared contract for one brick's bounded MCP contribution.
//!
//! This crate names no MCP transport, server framework, or async runtime. It
//! exists so a mature capability brick can implement [`HandlerContribution`]
//! without depending on `mcp-transport`, `rmcp`, or Tokio: those remain the
//! exclusive concern of the one project composition root that aggregates
//! selected contributions into a single unified MCP server process.
//!
//! [`HandlerContribution`] is intentionally object-safe (`dyn`-compatible) so
//! a project can hold a heterogeneous `Vec<Box<dyn HandlerContribution>>` of
//! brick contributions without generic parameterization over each brick's
//! concrete type. Async dispatch uses a manually boxed future rather than a
//! native `async fn` in the trait, because `async fn` in a trait is not
//! object-safe.
//!
//! A contribution owns its own private DTO conversion, semantic validation,
//! and (for every non-introspection tool) host-derived trusted-context
//! authorization. This trait never carries caller identity: MCP call
//! parameters are never a trust source, so `dispatch` accepts only bounded
//! JSON arguments and returns bounded JSON output.

use std::collections::BTreeMap;
use std::fmt;
use std::future::Future;
use std::pin::Pin;

use serde_json::{Map, Value};

/// Maximum bytes for a validated [`Namespace`] or the operation segment of a
/// [`ToolName`]. Matches the closed grammar `[a-z][a-z0-9_]{0,63}`.
pub const MAX_SEGMENT_BYTES: usize = 64;

/// The reserved namespace for project-level meta-tools. No brick
/// [`HandlerContribution`] may use this namespace.
pub const FACTORY_NAMESPACE: &str = "factory";

/// A closed `snake_case` identifier matching `[a-z][a-z0-9_]{0,63}`.
///
/// A brick's Cargo crate name normalizes hyphens to underscores before
/// validation, so `llm-gateway` yields the namespace `llm_gateway`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Namespace(String);

impl Namespace {
    /// Validates and constructs a namespace from an already-normalized value.
    ///
    /// Callers with a Cargo crate name should normalize hyphens to
    /// underscores before calling this constructor; normalization is not
    /// performed here so that an already-valid namespace is never silently
    /// rewritten.
    pub fn new(candidate: impl Into<String>) -> Result<Self, ContractError> {
        let candidate = candidate.into();
        if is_valid_segment(&candidate) {
            Ok(Self(candidate))
        } else {
            Err(ContractError::InvalidNamespace(candidate))
        }
    }

    /// Returns the reserved project-meta namespace, [`FACTORY_NAMESPACE`].
    #[must_use]
    pub fn factory_reserved() -> Self {
        Self(FACTORY_NAMESPACE.to_owned())
    }

    /// Returns whether this namespace is the reserved project-meta namespace.
    #[must_use]
    pub fn is_factory_reserved(&self) -> bool {
        self.0 == FACTORY_NAMESPACE
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Namespace {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A closed tool name of the form `<namespace>_<operation>`.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ToolName(String);

impl ToolName {
    /// Validates `operation` and joins it to `namespace` as
    /// `<namespace>_<operation>`.
    pub fn new(namespace: &Namespace, operation: &str) -> Result<Self, ContractError> {
        if !is_valid_segment(operation) {
            return Err(ContractError::InvalidOperation(operation.to_owned()));
        }
        let joined = format!("{namespace}_{operation}");
        if joined.len() > MAX_SEGMENT_BYTES * 2 + 1 {
            return Err(ContractError::InvalidOperation(operation.to_owned()));
        }
        Ok(Self(joined))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ToolName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

/// A validation failure for a [`Namespace`] or tool operation segment.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ContractError {
    InvalidNamespace(String),
    InvalidOperation(String),
}

impl fmt::Display for ContractError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidNamespace(value) => {
                write!(formatter, "invalid namespace: {value:?}")
            }
            Self::InvalidOperation(value) => {
                write!(formatter, "invalid tool operation: {value:?}")
            }
        }
    }
}

impl std::error::Error for ContractError {}

fn is_valid_segment(candidate: &str) -> bool {
    let mut bytes = candidate.bytes();
    candidate.len() <= MAX_SEGMENT_BYTES
        && matches!(bytes.next(), Some(byte) if byte.is_ascii_lowercase())
        && bytes.all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// One tool's bounded, self-describing metadata.
///
/// `input_schema` and `output_schema` are pre-generated (for example by
/// `schemars`) once per contribution; this crate names no schema-generation
/// dependency, so a contribution builds its own schema and hands over the
/// already-serialized closed JSON object.
#[derive(Clone, Debug)]
pub struct ToolDescriptor {
    pub name: ToolName,
    pub title: Option<String>,
    pub description: String,
    pub input_schema: Map<String, Value>,
    pub output_schema: Option<Map<String, Value>>,
}

/// The bounded outcome of one dispatched tool call.
///
/// `payload` is the contribution's own safe projection: it must never contain
/// internal error causes, secrets, trusted-context data, or raw Policy
/// artifacts. `is_error` distinguishes an ordinary tool-level failure
/// (malformed input, not-found, denied) from success; a routing or
/// infrastructure failure is instead reported as [`DispatchError`].
#[derive(Clone, Debug)]
pub struct DispatchOutcome {
    pub payload: Value,
    pub is_error: bool,
}

/// A dispatch failure that prevented the contribution from producing an
/// ordinary tool-level result.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DispatchError {
    /// The tool name is not owned by this contribution.
    UnknownTool,
    /// The arguments could not be interpreted. The message is a safe,
    /// bounded, caller-facing description; it must not repeat raw internal
    /// error text or secrets.
    MalformedArguments(String),
    /// An internal failure occurred while dispatching. No further detail is
    /// exposed to the caller.
    Internal,
}

impl fmt::Display for DispatchError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnknownTool => formatter.write_str("unknown tool"),
            Self::MalformedArguments(message) => {
                write!(formatter, "malformed arguments: {message}")
            }
            Self::Internal => formatter.write_str("internal dispatch failure"),
        }
    }
}

impl std::error::Error for DispatchError {}

/// A boxed future returned by [`HandlerContribution::dispatch`].
pub type DispatchFuture<'a> =
    Pin<Box<dyn Future<Output = Result<DispatchOutcome, DispatchError>> + Send + 'a>>;

/// One brick's bounded MCP contribution.
///
/// Implementations own DTO conversion, semantic validation, and (for every
/// non-introspection tool) host-derived trusted-context authorization
/// internally. A contribution never receives caller-supplied identity
/// through this trait: identity is bound once at construction time, mirroring
/// how an existing brick MCP handler is constructed with its trusted-context
/// source and Policy resolver today.
///
/// `tools()` is called once at aggregate construction; a contribution does
/// not add or remove tools at runtime. Implementations should include their
/// own `<namespace>_capabilities` and `<namespace>_schema` discovery tools in
/// the returned set, even when the contribution's only safe operational
/// surface is introspection.
pub trait HandlerContribution: Send + Sync {
    /// This contribution's stable namespace. Every tool name this
    /// contribution declares SHALL start with `<namespace()>_`.
    fn namespace(&self) -> &Namespace;

    /// The fixed, bounded set of tools this contribution exposes.
    fn tools(&self) -> Vec<ToolDescriptor>;

    /// Dispatches one call already routed to this contribution by tool name.
    fn dispatch(&self, tool: ToolName, arguments: Value) -> DispatchFuture<'_>;
}

/// Marker trait for a project-level meta-tool contribution.
///
/// A `ProjectMetaContribution`'s [`HandlerContribution::namespace`] SHALL
/// return [`Namespace::factory_reserved`]. Keeping this as a distinct trait
/// (rather than reusing [`HandlerContribution`] directly for project tools)
/// preserves the architectural distinction between brick-owned capability
/// tools and project-owned orchestration tools, even though both share one
/// dispatch mechanism.
pub trait ProjectMetaContribution: HandlerContribution {}

/// A read-only, ordered view of validated `<namespace, tool_name>` pairs.
///
/// Provided as a small convenience for a composition root that wants to log
/// or introspect what it registered without re-deriving names from each
/// contribution's [`ToolDescriptor`] list.
#[must_use]
pub fn tool_names_by_namespace(
    contributions: &[&dyn HandlerContribution],
) -> BTreeMap<String, Vec<String>> {
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for contribution in contributions {
        let namespace = contribution.namespace().as_str().to_owned();
        let names = grouped.entry(namespace).or_default();
        for tool in contribution.tools() {
            names.push(tool.name.as_str().to_owned());
        }
    }
    grouped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namespace_accepts_closed_snake_case_and_rejects_invalid_forms() {
        assert!(Namespace::new("agent").is_ok());
        assert!(Namespace::new("llm_gateway").is_ok());
        assert!(Namespace::new("a").is_ok());
        assert_eq!(
            Namespace::new("Agent"),
            Err(ContractError::InvalidNamespace("Agent".to_owned()))
        );
        assert!(Namespace::new("llm-gateway").is_err());
        assert!(Namespace::new("").is_err());
        assert!(Namespace::new("1agent").is_err());
        assert!(Namespace::new("a".repeat(65)).is_err());
        assert!(Namespace::new("a".repeat(64)).is_ok());
    }

    #[test]
    fn factory_reserved_namespace_is_stable_and_detected() {
        let namespace = Namespace::factory_reserved();
        assert_eq!(namespace.as_str(), FACTORY_NAMESPACE);
        assert!(namespace.is_factory_reserved());
        assert!(!Namespace::new("agent").unwrap().is_factory_reserved());
    }

    #[test]
    fn tool_name_joins_namespace_and_operation_and_rejects_invalid_operations() {
        let namespace = Namespace::new("agent").expect("namespace");
        let tool = ToolName::new(&namespace, "capabilities").expect("tool name");
        assert_eq!(tool.as_str(), "agent_capabilities");
        assert!(ToolName::new(&namespace, "Capabilities").is_err());
        assert!(ToolName::new(&namespace, "").is_err());
        assert!(ToolName::new(&namespace, "cap-abilities").is_err());
    }

    #[test]
    fn tool_names_by_namespace_groups_and_orders_deterministically() {
        struct Fixture {
            namespace: Namespace,
            names: Vec<ToolName>,
        }
        impl HandlerContribution for Fixture {
            fn namespace(&self) -> &Namespace {
                &self.namespace
            }
            fn tools(&self) -> Vec<ToolDescriptor> {
                self.names
                    .iter()
                    .map(|name| ToolDescriptor {
                        name: name.clone(),
                        title: None,
                        description: String::new(),
                        input_schema: Map::new(),
                        output_schema: None,
                    })
                    .collect()
            }
            fn dispatch(&self, _: ToolName, _: Value) -> DispatchFuture<'_> {
                Box::pin(async { Err(DispatchError::UnknownTool) })
            }
        }

        let agent_namespace = Namespace::new("agent").expect("namespace");
        let agent = Fixture {
            namespace: agent_namespace.clone(),
            names: vec![ToolName::new(&agent_namespace, "capabilities").expect("tool")],
        };
        let factory_namespace = Namespace::factory_reserved();
        let factory = Fixture {
            namespace: factory_namespace.clone(),
            names: vec![
                ToolName::new(&factory_namespace, "run_demo").expect("tool"),
                ToolName::new(&factory_namespace, "get_run").expect("tool"),
            ],
        };

        let grouped = tool_names_by_namespace(&[&agent as &dyn HandlerContribution, &factory]);
        assert_eq!(
            grouped.get("agent").map(Vec::as_slice),
            Some(["agent_capabilities".to_owned()].as_slice())
        );
        assert_eq!(
            grouped.get("factory").map(Vec::as_slice),
            Some(["factory_run_demo".to_owned(), "factory_get_run".to_owned()].as_slice())
        );
    }

    #[test]
    fn dispatch_future_resolves_to_bounded_outcome() {
        struct Echo(Namespace);
        impl HandlerContribution for Echo {
            fn namespace(&self) -> &Namespace {
                &self.0
            }
            fn tools(&self) -> Vec<ToolDescriptor> {
                vec![]
            }
            fn dispatch(&self, tool: ToolName, arguments: Value) -> DispatchFuture<'_> {
                Box::pin(async move {
                    if tool.as_str().ends_with("_echo") {
                        Ok(DispatchOutcome {
                            payload: arguments,
                            is_error: false,
                        })
                    } else {
                        Err(DispatchError::UnknownTool)
                    }
                })
            }
        }

        let namespace = Namespace::new("agent").expect("namespace");
        let echo = Echo(namespace.clone());
        let tool = ToolName::new(&namespace, "echo").expect("tool");
        let future = echo.dispatch(tool, Value::String("hi".to_owned()));
        let mut future = Box::pin(future);
        let waker = std::task::Waker::noop();
        let mut context = std::task::Context::from_waker(waker);
        match future.as_mut().poll(&mut context) {
            std::task::Poll::Ready(Ok(outcome)) => {
                assert!(!outcome.is_error);
                assert_eq!(outcome.payload, Value::String("hi".to_owned()));
            }
            other => panic!("expected immediate ready outcome, got {other:?}"),
        }
    }
}
