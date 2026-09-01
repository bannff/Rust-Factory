#![allow(clippy::missing_errors_doc, clippy::needless_pass_by_value)]

use anyhow::{Result, anyhow};
use mcp_contract::{
    DispatchError, DispatchFuture, DispatchOutcome, HandlerContribution, Namespace, ToolDescriptor,
    ToolName,
};
use policy::{
    AuthorizationDecisionV1, AuthorizationRequestV1, CapabilityV1, PolicyResolver,
    TrustedContextV1, canonical_grant, decision_digest,
};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router,
};
use schemars::{JsonSchema, schema_for};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{
    Command, CorrelationId, ExecuteRequest, ExecuteResult, PrincipalId, ProfileId, RequestId,
    Sandbox, SandboxContext, SandboxError, SandboxId, SandboxStatus, StartRequest, StartResult,
    StatusResult, StopResult, TargetRequest, TenantId,
};

const MAX_DTO_BYTES: usize = 16 * 1024;
const MAX_RESPONSE_BYTES: usize = 48 * 1024;
pub const SANDBOX_TOOLS: [&str; 6] = [
    "sandbox_capabilities",
    "sandbox_schema",
    "sandbox_start",
    "sandbox_execute",
    "sandbox_status",
    "sandbox_stop",
];

pub trait TrustedContextSource: Send + Sync {
    fn resolve(&self) -> Result<TrustedContextV1, SandboxError>;
}

pub struct SandboxMcp<S, T, P> {
    sandbox: S,
    context: T,
    policy: P,
    namespace: Namespace,
    tool_router: ToolRouter<Self>,
}
impl<S, T, P> SandboxMcp<S, T, P>
where
    S: Sandbox + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    /// Creates a server with injected Docker/MCP-independent adapters.
    ///
    /// # Panics
    ///
    /// Never panics in practice: the literal namespace `"sandbox"` always
    /// satisfies [`Namespace::new`]'s closed grammar.
    #[must_use]
    pub fn new(sandbox: S, context: T, policy: P) -> Self {
        Self {
            sandbox,
            context,
            policy,
            namespace: Namespace::new("sandbox").expect("literal namespace is valid"),
            tool_router: Self::tool_router(),
        }
    }

    fn authorize(&self, capability: CapabilityV1) -> Result<SandboxContext, SandboxError> {
        let trusted = self.context.resolve().map_err(|_| SandboxError::Denied)?;
        let request = AuthorizationRequestV1 {
            context: trusted.clone(),
            capability,
        };
        let AuthorizationDecisionV1::Allow {
            effective_grant,
            decision_digest: supplied,
        } = self.policy.authorize(request.clone())
        else {
            return Err(SandboxError::Denied);
        };
        let grant = canonical_grant(&effective_grant).map_err(|_| SandboxError::Denied)?;
        let expected = decision_digest(
            &request,
            &AuthorizationDecisionV1::Allow {
                effective_grant: grant,
                decision_digest: String::new(),
            },
        )
        .map_err(|_| SandboxError::Denied)?;
        if supplied != expected {
            return Err(SandboxError::Denied);
        }
        Ok(SandboxContext {
            tenant_id: TenantId::new(trusted.tenant_id.as_str())
                .map_err(|_| SandboxError::Denied)?,
            principal_id: PrincipalId::new(trusted.principal_id.as_str())
                .map_err(|_| SandboxError::Denied)?,
            request_id: RequestId::new(trusted.request_id.as_str())
                .map_err(|_| SandboxError::Denied)?,
            correlation_id: CorrelationId::new(trusted.correlation_id.as_str())
                .map_err(|_| SandboxError::Denied)?,
        })
    }

    fn start_json(&self, input: StartInput) -> Result<String> {
        check_size(&input)?;
        let profile_id = ProfileId::new(input.profile).map_err(public)?;
        let context = self.authorize(CapabilityV1::SandboxStart).map_err(public)?;
        project_start(
            self.sandbox
                .start(StartRequest {
                    context,
                    profile_id,
                })
                .map_err(public)?,
        )
    }
    fn execute_json(&self, input: ExecuteInput) -> Result<String> {
        check_size(&input)?;
        let sandbox_id = SandboxId::new(input.sandbox_id).map_err(public)?;
        let command = Command::new(
            input.program,
            input.arguments,
            input.working_directory,
            input.timeout_millis,
        )
        .map_err(public)?;
        let context = self
            .authorize(CapabilityV1::SandboxExecute)
            .map_err(public)?;
        project_execute(
            self.sandbox
                .execute(ExecuteRequest {
                    context,
                    sandbox_id,
                    command,
                })
                .map_err(public)?,
        )
    }
    fn status_json(&self, input: TargetInput) -> Result<String> {
        check_size(&input)?;
        let sandbox_id = SandboxId::new(input.sandbox_id).map_err(public)?;
        let context = self
            .authorize(CapabilityV1::SandboxStatus)
            .map_err(public)?;
        project_status(
            self.sandbox
                .status(TargetRequest {
                    context,
                    sandbox_id,
                })
                .map_err(public)?,
        )
    }
    fn stop_json(&self, input: TargetInput) -> Result<String> {
        check_size(&input)?;
        let sandbox_id = SandboxId::new(input.sandbox_id).map_err(public)?;
        let context = self.authorize(CapabilityV1::SandboxStop).map_err(public)?;
        project_stop(
            self.sandbox
                .stop(TargetRequest {
                    context,
                    sandbox_id,
                })
                .map_err(public)?,
        )
    }
    #[allow(
        clippy::unused_self,
        reason = "kept as &self for uniformity with the other *_json methods dispatch() calls"
    )]
    fn capabilities_json(&self, input: EmptyInput) -> Result<String> {
        check_size(&input)?;
        serialize(&json!({
            "operations": ["start", "execute", "status", "stop"],
            "durable": false, "retries": false, "recovery": false,
        }))
    }
    #[allow(
        clippy::unused_self,
        reason = "kept as &self for uniformity with the other *_json methods dispatch() calls"
    )]
    fn schema_json(&self, input: EmptyInput) -> Result<String> {
        check_size(&input)?;
        serialize(&json!({
            "requests": {"start": schema_for!(StartInput), "execute": schema_for!(ExecuteInput), "status": schema_for!(TargetInput), "stop": schema_for!(TargetInput)},
            "responses": {"start": schema_for!(StartOutput), "execute": schema_for!(ExecuteOutput), "status": schema_for!(StatusOutput), "stop": schema_for!(StopOutput)},
        }))
    }
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct EmptyInput {}
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct StartInput {
    profile: String,
}
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct ExecuteInput {
    sandbox_id: String,
    program: String,
    arguments: Vec<String>,
    #[serde(default)]
    working_directory: String,
    timeout_millis: u64,
}
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct TargetInput {
    sandbox_id: String,
}

#[derive(JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct StartOutput {
    sandbox_id: String,
    status: &'static str,
}
#[derive(JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct ExecuteOutput {
    sandbox_id: String,
    exit_code: i32,
    stdout: String,
    stderr: String,
    truncated: bool,
}
#[derive(JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct StatusOutput {
    sandbox_id: String,
    status: &'static str,
}
#[derive(JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
struct StopOutput {
    sandbox_id: String,
    removed: bool,
}

#[tool_router(router = tool_router)]
impl<S, T, P> SandboxMcp<S, T, P>
where
    S: Sandbox + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    #[tool(
        name = "sandbox_capabilities",
        description = "Describe the minimal Sandbox operations and guarantees."
    )]
    async fn capabilities(&self, Parameters(input): Parameters<EmptyInput>) -> String {
        tool_response(self.capabilities_json(input))
    }
    #[tool(
        name = "sandbox_schema",
        description = "Return generated closed request and response schemas."
    )]
    async fn schema(&self, Parameters(input): Parameters<EmptyInput>) -> String {
        tool_response(self.schema_json(input))
    }
    #[tool(
        name = "sandbox_start",
        description = "Start one trusted ephemeral sandbox profile."
    )]
    async fn start(&self, Parameters(input): Parameters<StartInput>) -> String {
        tool_response(self.start_json(input))
    }
    #[tool(
        name = "sandbox_execute",
        description = "Execute one bounded command in an owned sandbox."
    )]
    async fn execute(&self, Parameters(input): Parameters<ExecuteInput>) -> String {
        tool_response(self.execute_json(input))
    }
    #[tool(
        name = "sandbox_status",
        description = "Read one owned sandbox status."
    )]
    async fn status(&self, Parameters(input): Parameters<TargetInput>) -> String {
        tool_response(self.status_json(input))
    }
    #[tool(
        name = "sandbox_stop",
        description = "Stop and remove one owned sandbox."
    )]
    async fn stop(&self, Parameters(input): Parameters<TargetInput>) -> String {
        tool_response(self.stop_json(input))
    }
}

// `#[tool_handler]` (rmcp) expands into `async fn` trait impl methods whose
// bodies contain no `.await` (they resolve immediately via
// `std::future::ready`-equivalent machinery). A newer Clippy version's
// `unused_async_trait_impl` lint flags this macro-generated code; the
// generated functions are not something this crate can rewrite directly.
// Suppressed at the macro invocation site pending an rmcp upstream fix.
#[allow(unknown_lints, clippy::unused_async_trait_impl)]
#[tool_handler(router = self.tool_router)]
impl<S, T, P> ServerHandler for SandboxMcp<S, T, P>
where
    S: Sandbox + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
}

/// This contribution declares its full six-tool public surface with no
/// exclusions. Unlike `project`'s `HandlerContribution` migration, which
/// deliberately excludes `project_generate` pending a `TrustedContextSource`/
/// `PolicyResolver` authorization gate, every one of Sandbox's four
/// effectful tools already authorizes via [`SandboxMcp::authorize`] before
/// any effect on the standalone `ServerHandler` path, and `dispatch` below
/// reuses the exact same `*_json` methods that path calls — so there is no
/// analogous gap to gate around.
impl<S, T, P> HandlerContribution for SandboxMcp<S, T, P>
where
    S: Sandbox + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    fn namespace(&self) -> &Namespace {
        &self.namespace
    }

    fn tools(&self) -> Vec<ToolDescriptor> {
        let namespace = &self.namespace;
        vec![
            descriptor(
                namespace,
                "start",
                "Start one trusted ephemeral sandbox profile.",
                schema_map::<StartInput>(),
            ),
            descriptor(
                namespace,
                "execute",
                "Execute one bounded command in an owned sandbox.",
                schema_map::<ExecuteInput>(),
            ),
            descriptor(
                namespace,
                "status",
                "Read one owned sandbox status.",
                schema_map::<TargetInput>(),
            ),
            descriptor(
                namespace,
                "stop",
                "Stop and remove one owned sandbox.",
                schema_map::<TargetInput>(),
            ),
            descriptor(
                namespace,
                "capabilities",
                "Describe the minimal Sandbox operations and guarantees.",
                schema_map::<EmptyInput>(),
            ),
            descriptor(
                namespace,
                "schema",
                "Return generated closed request and response schemas.",
                schema_map::<EmptyInput>(),
            ),
        ]
    }

    fn dispatch(&self, tool: ToolName, arguments: serde_json::Value) -> DispatchFuture<'_> {
        Box::pin(async move {
            match tool.as_str() {
                "sandbox_start" => {
                    let input = deserialize_arguments(arguments)?;
                    Ok(fold_outcome(self.start_json(input)))
                }
                "sandbox_execute" => {
                    let input = deserialize_arguments(arguments)?;
                    Ok(fold_outcome(self.execute_json(input)))
                }
                "sandbox_status" => {
                    let input = deserialize_arguments(arguments)?;
                    Ok(fold_outcome(self.status_json(input)))
                }
                "sandbox_stop" => {
                    let input = deserialize_arguments(arguments)?;
                    Ok(fold_outcome(self.stop_json(input)))
                }
                "sandbox_capabilities" => {
                    let input = deserialize_arguments(arguments)?;
                    Ok(fold_outcome(self.capabilities_json(input)))
                }
                "sandbox_schema" => {
                    let input = deserialize_arguments(arguments)?;
                    Ok(fold_outcome(self.schema_json(input)))
                }
                _ => Err(DispatchError::UnknownTool),
            }
        })
    }
}

fn schema_map<V: JsonSchema>() -> serde_json::Map<String, serde_json::Value> {
    serde_json::to_value(schema_for!(V))
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

fn deserialize_arguments<V: serde::de::DeserializeOwned>(
    arguments: serde_json::Value,
) -> std::result::Result<V, DispatchError> {
    serde_json::from_value(arguments)
        .map_err(|_| DispatchError::MalformedArguments("could not parse tool arguments".to_owned()))
}

/// Reuses [`tool_response`]'s exact safe-code allowlist so the granular
/// redacted error taxonomy already proven by `public_errors_are_redacted`
/// and the authorization-ordering tests is identical between the standalone
/// `ServerHandler` path and this `HandlerContribution` path. This
/// deliberately does not collapse every failure to one fallback code the way
/// `project`'s `fold_outcome` does, since `project`'s standalone
/// `tool_response` distinguishes only success from `"generation_failed"`,
/// while Sandbox's already distinguishes `denied`/`limit_exceeded`/
/// `timeout`/`not_found`/`invalid_request`/`outcome_unknown` as safe codes.
fn fold_outcome(result: Result<String>) -> DispatchOutcome {
    let projected = tool_response(result);
    DispatchOutcome {
        payload: serde_json::from_str(&projected)
            .unwrap_or_else(|_| json!({ "error": "operation_failed" })),
        is_error: false,
    }
}

fn status(value: SandboxStatus) -> &'static str {
    match value {
        SandboxStatus::Running => "running",
        SandboxStatus::Stopped => "stopped",
    }
}
fn project_start(value: StartResult) -> Result<String> {
    serialize(&StartOutput {
        sandbox_id: value.sandbox_id.as_str().into(),
        status: status(value.status),
    })
}
fn project_execute(value: ExecuteResult) -> Result<String> {
    serialize(&ExecuteOutput {
        sandbox_id: value.sandbox_id.as_str().into(),
        exit_code: value.exit_code,
        stdout: value.stdout,
        stderr: value.stderr,
        truncated: value.truncated,
    })
}
fn project_status(value: StatusResult) -> Result<String> {
    serialize(&StatusOutput {
        sandbox_id: value.sandbox_id.as_str().into(),
        status: status(value.status),
    })
}
fn project_stop(value: StopResult) -> Result<String> {
    serialize(&StopOutput {
        sandbox_id: value.sandbox_id.as_str().into(),
        removed: value.removed,
    })
}
fn check_size<T: Serialize>(value: &T) -> Result<()> {
    (serde_json::to_vec(value)?.len() <= MAX_DTO_BYTES)
        .then_some(())
        .ok_or_else(|| anyhow!("limit_exceeded"))
}
fn serialize<T: Serialize>(value: &T) -> Result<String> {
    let text = serde_json::to_string(value)?;
    (text.len() <= MAX_RESPONSE_BYTES)
        .then_some(text)
        .ok_or_else(|| anyhow!("limit_exceeded"))
}
fn public(error: SandboxError) -> anyhow::Error {
    anyhow!(error.as_str())
}
fn tool_response(result: Result<String>) -> String {
    result.unwrap_or_else(|error| {
        let code = error.to_string();
        let safe = match code.as_str() {
            "invalid_request" | "not_found" | "denied" | "limit_exceeded" | "timeout"
            | "unavailable" | "outcome_unknown" => code,
            _ => "operation_failed".into(),
        };
        json!({"error": safe}).to_string()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_dtos_are_closed_and_reject_unknown_fields() {
        assert!(
            serde_json::from_value::<StartInput>(json!({"profile":"rust","extra":true})).is_err()
        );
        assert!(
            serde_json::from_value::<ExecuteInput>(json!({
                "sandbox_id": format!("sbx-{}", "a".repeat(32)),
                "program":"cargo", "arguments":[], "timeout_millis":1000, "extra":true
            }))
            .is_err()
        );
        for schema in [
            schema_for!(StartInput),
            schema_for!(ExecuteInput),
            schema_for!(TargetInput),
        ] {
            let value = serde_json::to_value(schema).unwrap();
            assert_eq!(value["additionalProperties"], false);
        }
    }

    #[test]
    fn start_json_rejects_oversized_dto_before_authorization() {
        struct DenyContext;
        impl TrustedContextSource for DenyContext {
            fn resolve(&self) -> Result<policy::TrustedContextV1, SandboxError> {
                panic!("must not be reached: size check must short-circuit first")
            }
        }
        struct DenyPolicy;
        impl PolicyResolver for DenyPolicy {
            fn authorize(&self, _: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
                panic!("must not be reached: size check must short-circuit first")
            }
        }
        struct DenySandboxAdapter;
        impl Sandbox for DenySandboxAdapter {
            fn start(&self, _: StartRequest) -> std::result::Result<StartResult, SandboxError> {
                unreachable!()
            }
            fn execute(
                &self,
                _: ExecuteRequest,
            ) -> std::result::Result<ExecuteResult, SandboxError> {
                unreachable!()
            }
            fn status(&self, _: TargetRequest) -> std::result::Result<StatusResult, SandboxError> {
                unreachable!()
            }
            fn stop(&self, _: TargetRequest) -> std::result::Result<StopResult, SandboxError> {
                unreachable!()
            }
        }
        let mcp = SandboxMcp::new(DenySandboxAdapter, DenyContext, DenyPolicy);
        let oversized = StartInput {
            profile: "x".repeat(MAX_DTO_BYTES + 1),
        };
        let result = mcp.start_json(oversized);
        assert!(result.is_err(), "oversized StartInput must be rejected");
    }

    #[test]
    fn execute_json_rejects_invalid_sandbox_id_before_authorization() {
        struct PanicContext;
        impl TrustedContextSource for PanicContext {
            fn resolve(&self) -> Result<policy::TrustedContextV1, SandboxError> {
                panic!("must not be reached: sandbox_id validation must short-circuit first")
            }
        }
        struct PanicPolicy;
        impl PolicyResolver for PanicPolicy {
            fn authorize(&self, _: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
                panic!("must not be reached")
            }
        }
        struct PanicSandbox;
        impl Sandbox for PanicSandbox {
            fn start(&self, _: StartRequest) -> std::result::Result<StartResult, SandboxError> {
                unreachable!()
            }
            fn execute(
                &self,
                _: ExecuteRequest,
            ) -> std::result::Result<ExecuteResult, SandboxError> {
                unreachable!()
            }
            fn status(&self, _: TargetRequest) -> std::result::Result<StatusResult, SandboxError> {
                unreachable!()
            }
            fn stop(&self, _: TargetRequest) -> std::result::Result<StopResult, SandboxError> {
                unreachable!()
            }
        }
        let mcp = SandboxMcp::new(PanicSandbox, PanicContext, PanicPolicy);
        for bad_id in ["", "not-a-sandbox-id", "sbx-tooshort", "../etc/passwd"] {
            let input = ExecuteInput {
                sandbox_id: bad_id.into(),
                program: "cargo".into(),
                arguments: vec![],
                working_directory: String::new(),
                timeout_millis: 1_000,
            };
            let result = mcp.execute_json(input);
            assert!(
                result.is_err(),
                "malformed sandbox_id {bad_id:?} must be rejected before authorization"
            );
        }
    }

    #[test]
    fn execute_json_rejects_command_exceeding_bounds() {
        struct PanicContext;
        impl TrustedContextSource for PanicContext {
            fn resolve(&self) -> Result<policy::TrustedContextV1, SandboxError> {
                panic!("must not be reached: Command validation must short-circuit first")
            }
        }
        struct PanicPolicy;
        impl PolicyResolver for PanicPolicy {
            fn authorize(&self, _: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
                panic!("must not be reached")
            }
        }
        struct PanicSandbox;
        impl Sandbox for PanicSandbox {
            fn start(&self, _: StartRequest) -> std::result::Result<StartResult, SandboxError> {
                unreachable!()
            }
            fn execute(
                &self,
                _: ExecuteRequest,
            ) -> std::result::Result<ExecuteResult, SandboxError> {
                unreachable!()
            }
            fn status(&self, _: TargetRequest) -> std::result::Result<StatusResult, SandboxError> {
                unreachable!()
            }
            fn stop(&self, _: TargetRequest) -> std::result::Result<StopResult, SandboxError> {
                unreachable!()
            }
        }
        let mcp = SandboxMcp::new(PanicSandbox, PanicContext, PanicPolicy);
        let too_many_arguments = ExecuteInput {
            sandbox_id: format!("sbx-{}", "a".repeat(32)),
            program: "cargo".into(),
            arguments: vec!["x".into(); crate::MAX_ARGUMENTS + 1],
            working_directory: String::new(),
            timeout_millis: 1_000,
        };
        assert!(mcp.execute_json(too_many_arguments).is_err());

        let excessive_timeout = ExecuteInput {
            sandbox_id: format!("sbx-{}", "a".repeat(32)),
            program: "cargo".into(),
            arguments: vec![],
            working_directory: String::new(),
            timeout_millis: crate::MAX_TIMEOUT_MILLIS + 1,
        };
        assert!(mcp.execute_json(excessive_timeout).is_err());
    }

    #[test]
    fn public_errors_are_redacted() {
        assert_eq!(
            tool_response(Err(anyhow!("/private/docker.sock secret"))),
            r#"{"error":"operation_failed"}"#
        );
        assert_eq!(
            tool_response(Err(anyhow!("denied"))),
            r#"{"error":"denied"}"#
        );
    }

    #[test]
    fn declared_tool_surface_is_exact() {
        assert_eq!(
            SANDBOX_TOOLS,
            [
                "sandbox_capabilities",
                "sandbox_schema",
                "sandbox_start",
                "sandbox_execute",
                "sandbox_status",
                "sandbox_stop"
            ]
        );
    }

    struct AllowContext;
    impl TrustedContextSource for AllowContext {
        fn resolve(&self) -> Result<policy::TrustedContextV1, SandboxError> {
            Ok(policy::TrustedContextV1 {
                tenant_id: policy::TenantId::new("tenant").unwrap(),
                principal_id: policy::PrincipalId::new("principal").unwrap(),
                request_id: policy::RequestId::new("request").unwrap(),
                correlation_id: policy::CorrelationId::new("correlation").unwrap(),
            })
        }
    }
    struct AllowPolicy;
    impl PolicyResolver for AllowPolicy {
        fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
            let grant =
                policy::GrantV1::new(Vec::<String>::new(), false, false, true, false).unwrap();
            let digest = decision_digest(
                &request,
                &AuthorizationDecisionV1::Allow {
                    effective_grant: grant.clone(),
                    decision_digest: String::new(),
                },
            )
            .unwrap();
            AuthorizationDecisionV1::Allow {
                effective_grant: grant,
                decision_digest: digest,
            }
        }
    }
    struct StubSandbox;
    impl Sandbox for StubSandbox {
        fn start(&self, _: StartRequest) -> std::result::Result<StartResult, SandboxError> {
            Ok(StartResult {
                sandbox_id: SandboxId::new(format!("sbx-{}", "a".repeat(32))).unwrap(),
                status: SandboxStatus::Running,
            })
        }
        fn execute(&self, _: ExecuteRequest) -> std::result::Result<ExecuteResult, SandboxError> {
            unreachable!("not exercised by this test")
        }
        fn status(
            &self,
            request: TargetRequest,
        ) -> std::result::Result<StatusResult, SandboxError> {
            Ok(StatusResult {
                sandbox_id: request.sandbox_id,
                status: SandboxStatus::Running,
            })
        }
        fn stop(&self, _: TargetRequest) -> std::result::Result<StopResult, SandboxError> {
            unreachable!("not exercised by this test")
        }
    }

    #[test]
    fn handler_contribution_namespace_is_sandbox() {
        let mcp = SandboxMcp::new(StubSandbox, AllowContext, AllowPolicy);
        assert_eq!(HandlerContribution::namespace(&mcp).as_str(), "sandbox");
    }

    #[test]
    fn handler_contribution_tools_are_the_full_six_with_no_exclusions() {
        let mcp = SandboxMcp::new(StubSandbox, AllowContext, AllowPolicy);
        let mut names: Vec<String> = HandlerContribution::tools(&mcp)
            .into_iter()
            .map(|tool| tool.name.as_str().to_owned())
            .collect();
        names.sort();
        let mut expected: Vec<String> = SANDBOX_TOOLS.iter().map(|&name| name.to_owned()).collect();
        expected.sort();
        assert_eq!(
            names, expected,
            "HandlerContribution::tools() must declare every SANDBOX_TOOLS entry with no exclusions, \
             unlike project's project_generate exclusion"
        );
    }

    #[test]
    fn handler_contribution_dispatch_rejects_unknown_fields() {
        let mcp = SandboxMcp::new(StubSandbox, AllowContext, AllowPolicy);
        let namespace = Namespace::new("sandbox").unwrap();
        let tool = ToolName::new(&namespace, "status").unwrap();
        let arguments = json!({"sandbox_id": format!("sbx-{}", "a".repeat(32)), "extra": true});
        let error = futures_executor_block(mcp.dispatch(tool, arguments))
            .expect_err("unknown field must be rejected before dispatch succeeds");
        assert!(matches!(error, DispatchError::MalformedArguments(_)));
    }

    #[test]
    fn handler_contribution_dispatch_rejects_unknown_tool() {
        let mcp = SandboxMcp::new(StubSandbox, AllowContext, AllowPolicy);
        let namespace = Namespace::new("sandbox").unwrap();
        let tool = ToolName::new(&namespace, "does_not_exist").unwrap();
        let error = futures_executor_block(mcp.dispatch(tool, json!({})))
            .expect_err("unknown tool must be rejected");
        assert_eq!(error, DispatchError::UnknownTool);
    }

    #[test]
    fn handler_contribution_dispatch_status_matches_standalone_tool_response() {
        let mcp = SandboxMcp::new(StubSandbox, AllowContext, AllowPolicy);
        let namespace = Namespace::new("sandbox").unwrap();
        let tool = ToolName::new(&namespace, "status").unwrap();
        let sandbox_id = format!("sbx-{}", "a".repeat(32));
        let arguments = json!({"sandbox_id": sandbox_id});

        let standalone = mcp
            .status_json(TargetInput {
                sandbox_id: sandbox_id.clone(),
            })
            .expect("standalone status_json succeeds");
        let standalone_value: serde_json::Value =
            serde_json::from_str(&standalone).expect("standalone output is JSON");

        let outcome =
            futures_executor_block(mcp.dispatch(tool, arguments)).expect("dispatch succeeds");
        assert!(!outcome.is_error);
        assert_eq!(
            outcome.payload, standalone_value,
            "HandlerContribution::dispatch must reuse the same *_json method as the standalone tool"
        );
    }

    #[test]
    fn handler_contribution_dispatch_capabilities_and_schema_are_bounded_and_safe() {
        let mcp = SandboxMcp::new(StubSandbox, AllowContext, AllowPolicy);
        let namespace = Namespace::new("sandbox").unwrap();

        let capabilities_tool = ToolName::new(&namespace, "capabilities").unwrap();
        let capabilities = futures_executor_block(mcp.dispatch(capabilities_tool, json!({})))
            .expect("capabilities dispatch succeeds");
        assert!(!capabilities.is_error);
        assert_eq!(capabilities.payload["operations"][0], "start");

        let schema_tool = ToolName::new(&namespace, "schema").unwrap();
        let schema = futures_executor_block(mcp.dispatch(schema_tool, json!({})))
            .expect("schema dispatch succeeds");
        assert!(!schema.is_error);
        assert!(schema.payload["requests"]["start"].is_object());
    }

    #[test]
    fn handler_contribution_dispatch_preserves_granular_redacted_error_taxonomy() {
        // A malformed sandbox_id must surface the same safe "invalid_request"
        // code on the unified dispatch path as it does on the standalone
        // ServerHandler path, rather than collapsing to a coarser fallback.
        let mcp = SandboxMcp::new(StubSandbox, AllowContext, AllowPolicy);
        let namespace = Namespace::new("sandbox").unwrap();
        let tool = ToolName::new(&namespace, "status").unwrap();
        let arguments = json!({"sandbox_id": "not-a-valid-id"});

        let outcome = futures_executor_block(mcp.dispatch(tool, arguments))
            .expect("dispatch itself succeeds; the failure is tool-level");
        assert_eq!(outcome.payload["error"], "invalid_request");
    }

    /// Polls a `DispatchFuture` to completion without pulling in an async
    /// test runtime dependency; every dispatch path exercised by these tests
    /// resolves immediately (no adapter here performs real async I/O).
    fn futures_executor_block<F: std::future::Future>(future: F) -> F::Output {
        let mut future = Box::pin(future);
        let waker = std::task::Waker::noop();
        let mut context = std::task::Context::from_waker(waker);
        loop {
            if let std::task::Poll::Ready(value) = future.as_mut().poll(&mut context) {
                return value;
            }
        }
    }

    /// A `Sandbox` stub that returns real, non-panicking success values for
    /// all four effectful operations. `StubSandbox` above only implements
    /// `start`/`status` (the other two `unreachable!()`), which is
    /// insufficient to prove dispatch-vs-standalone parity for
    /// `execute`/`stop`.
    struct FullStubSandbox;
    impl Sandbox for FullStubSandbox {
        fn start(&self, _: StartRequest) -> std::result::Result<StartResult, SandboxError> {
            Ok(StartResult {
                sandbox_id: SandboxId::new(format!("sbx-{}", "b".repeat(32))).unwrap(),
                status: SandboxStatus::Running,
            })
        }
        fn execute(
            &self,
            request: ExecuteRequest,
        ) -> std::result::Result<ExecuteResult, SandboxError> {
            Ok(ExecuteResult {
                sandbox_id: request.sandbox_id,
                exit_code: 0,
                stdout: "ok".into(),
                stderr: String::new(),
                truncated: false,
            })
        }
        fn status(
            &self,
            request: TargetRequest,
        ) -> std::result::Result<StatusResult, SandboxError> {
            Ok(StatusResult {
                sandbox_id: request.sandbox_id,
                status: SandboxStatus::Running,
            })
        }
        fn stop(&self, request: TargetRequest) -> std::result::Result<StopResult, SandboxError> {
            Ok(StopResult {
                sandbox_id: request.sandbox_id,
                removed: true,
            })
        }
    }

    /// Regression for adversarial finding: only `status` had a
    /// dispatch-vs-standalone parity test, and only `status`/`capabilities`/
    /// `schema` were proven genuinely dispatchable (not silently returning
    /// `UnknownTool`). This proves all three remaining effectful tools
    /// (`start`, `execute`, `stop`) are dispatchable and byte-for-byte
    /// identical to their standalone `*_json` output.
    #[test]
    fn handler_contribution_dispatch_start_execute_stop_match_standalone_and_are_reachable() {
        let mcp = SandboxMcp::new(FullStubSandbox, AllowContext, AllowPolicy);
        let namespace = Namespace::new("sandbox").unwrap();

        let start_input = StartInput {
            profile: "rust".into(),
        };
        let standalone: serde_json::Value =
            serde_json::from_str(&mcp.start_json(start_input.clone()).unwrap()).unwrap();
        let tool = ToolName::new(&namespace, "start").unwrap();
        let arguments = serde_json::to_value(&start_input).unwrap();
        let outcome = futures_executor_block(mcp.dispatch(tool, arguments))
            .expect("sandbox_start must be genuinely dispatchable, not UnknownTool");
        assert!(!outcome.is_error);
        assert_eq!(outcome.payload, standalone);

        let execute_input = ExecuteInput {
            sandbox_id: format!("sbx-{}", "b".repeat(32)),
            program: "cargo".into(),
            arguments: vec!["test".into()],
            working_directory: String::new(),
            timeout_millis: 1_000,
        };
        let standalone: serde_json::Value =
            serde_json::from_str(&mcp.execute_json(execute_input.clone()).unwrap()).unwrap();
        let tool = ToolName::new(&namespace, "execute").unwrap();
        let arguments = serde_json::to_value(&execute_input).unwrap();
        let outcome = futures_executor_block(mcp.dispatch(tool, arguments))
            .expect("sandbox_execute must be genuinely dispatchable, not UnknownTool");
        assert!(!outcome.is_error);
        assert_eq!(outcome.payload, standalone);

        let stop_input = TargetInput {
            sandbox_id: format!("sbx-{}", "b".repeat(32)),
        };
        let standalone: serde_json::Value =
            serde_json::from_str(&mcp.stop_json(stop_input.clone()).unwrap()).unwrap();
        let tool = ToolName::new(&namespace, "stop").unwrap();
        let arguments = serde_json::to_value(&stop_input).unwrap();
        let outcome = futures_executor_block(mcp.dispatch(tool, arguments))
            .expect("sandbox_stop must be genuinely dispatchable, not UnknownTool");
        assert!(!outcome.is_error);
        assert_eq!(outcome.payload, standalone);
    }

    /// Regression for adversarial finding: the only existing
    /// authorization-ordering-through-`dispatch()` proof used `status`
    /// (`handler_contribution_dispatch_preserves_granular_redacted_error_taxonomy`),
    /// and no test proved that an oversized DTO on the *effectful* `start`
    /// path short-circuits before `authorize()`/the port is ever reached
    /// when driven through `dispatch()` rather than the private method
    /// directly.
    #[test]
    fn handler_contribution_dispatch_rejects_oversized_start_before_authorization() {
        struct DenyContext;
        impl TrustedContextSource for DenyContext {
            fn resolve(&self) -> Result<policy::TrustedContextV1, SandboxError> {
                panic!("must not be reached: size check must short-circuit first")
            }
        }
        struct DenyPolicy;
        impl PolicyResolver for DenyPolicy {
            fn authorize(&self, _: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
                panic!("must not be reached: size check must short-circuit first")
            }
        }
        struct DenySandboxAdapter;
        impl Sandbox for DenySandboxAdapter {
            fn start(&self, _: StartRequest) -> std::result::Result<StartResult, SandboxError> {
                unreachable!()
            }
            fn execute(
                &self,
                _: ExecuteRequest,
            ) -> std::result::Result<ExecuteResult, SandboxError> {
                unreachable!()
            }
            fn status(&self, _: TargetRequest) -> std::result::Result<StatusResult, SandboxError> {
                unreachable!()
            }
            fn stop(&self, _: TargetRequest) -> std::result::Result<StopResult, SandboxError> {
                unreachable!()
            }
        }
        let mcp = SandboxMcp::new(DenySandboxAdapter, DenyContext, DenyPolicy);
        let namespace = Namespace::new("sandbox").unwrap();
        let tool = ToolName::new(&namespace, "start").unwrap();
        let oversized = StartInput {
            profile: "x".repeat(MAX_DTO_BYTES + 1),
        };
        let arguments = serde_json::to_value(&oversized).unwrap();
        let outcome = futures_executor_block(mcp.dispatch(tool, arguments))
            .expect("dispatch itself succeeds; the oversized DTO is a tool-level failure");
        assert_eq!(
            outcome.payload["error"], "limit_exceeded",
            "oversized DTO must be rejected by check_size before DenyContext/DenyPolicy/DenySandboxAdapter are ever touched"
        );
    }

    /// A `Sandbox` stub whose every operation fails with the same
    /// caller-configured `SandboxError`, used to prove `dispatch()`
    /// preserves each of `tool_response`'s safe codes, not just
    /// `invalid_request` (the only variant the pre-existing
    /// `handler_contribution_dispatch_preserves_granular_redacted_error_taxonomy`
    /// test covered).
    struct ErrorSandbox(SandboxError);
    impl Sandbox for ErrorSandbox {
        fn start(&self, _: StartRequest) -> std::result::Result<StartResult, SandboxError> {
            Err(self.0)
        }
        fn execute(&self, _: ExecuteRequest) -> std::result::Result<ExecuteResult, SandboxError> {
            Err(self.0)
        }
        fn status(&self, _: TargetRequest) -> std::result::Result<StatusResult, SandboxError> {
            Err(self.0)
        }
        fn stop(&self, _: TargetRequest) -> std::result::Result<StopResult, SandboxError> {
            Err(self.0)
        }
    }

    #[test]
    fn handler_contribution_dispatch_preserves_full_error_taxonomy() {
        let namespace = Namespace::new("sandbox").unwrap();
        for (error, expected_code) in [
            (SandboxError::NotFound, "not_found"),
            (SandboxError::Denied, "denied"),
            (SandboxError::LimitExceeded, "limit_exceeded"),
            (SandboxError::Timeout, "timeout"),
            (SandboxError::Unavailable, "unavailable"),
            (SandboxError::OutcomeUnknown, "outcome_unknown"),
            (SandboxError::OperationFailed, "operation_failed"),
        ] {
            let mcp = SandboxMcp::new(ErrorSandbox(error), AllowContext, AllowPolicy);
            let tool = ToolName::new(&namespace, "status").unwrap();
            let arguments = json!({"sandbox_id": format!("sbx-{}", "a".repeat(32))});
            let outcome = futures_executor_block(mcp.dispatch(tool, arguments))
                .expect("dispatch succeeds; the port failure is a tool-level outcome");
            assert_eq!(
                outcome.payload["error"], expected_code,
                "SandboxError::{error:?} must surface as {expected_code:?} through dispatch(), \
                 identical to the standalone tool_response path"
            );
        }
    }

    /// Regression for adversarial finding: nothing proved the
    /// `schema_map::<V>()` used by `HandlerContribution::tools()`'s
    /// `input_schema` actually matches the `schema_for!(V)` the standalone
    /// `sandbox_schema` tool embeds. A drift here (e.g. one path adding a
    /// field the other lacks) would silently desynchronize what an agent
    /// discovers via `tools()` from what it gets calling `sandbox_schema`.
    #[test]
    fn handler_contribution_tools_input_schema_matches_standalone_schema_generation() {
        let mcp = SandboxMcp::new(StubSandbox, AllowContext, AllowPolicy);
        let tools = HandlerContribution::tools(&mcp);

        let start_descriptor = tools
            .iter()
            .find(|tool| tool.name.as_str() == "sandbox_start")
            .expect("sandbox_start must be declared");
        let expected_start = serde_json::to_value(schema_for!(StartInput)).unwrap();
        assert_eq!(
            &start_descriptor.input_schema,
            expected_start.as_object().unwrap()
        );

        let standalone_schema: serde_json::Value =
            serde_json::from_str(&mcp.schema_json(EmptyInput {}).unwrap()).unwrap();
        assert_eq!(
            standalone_schema["requests"]["start"], expected_start,
            "tools()'s schema_map::<StartInput>() must not drift from schema_for!(StartInput) \
             embedded in the standalone sandbox_schema tool"
        );

        let target_descriptor = tools
            .iter()
            .find(|tool| tool.name.as_str() == "sandbox_status")
            .expect("sandbox_status must be declared");
        let expected_target = serde_json::to_value(schema_for!(TargetInput)).unwrap();
        assert_eq!(
            &target_descriptor.input_schema,
            expected_target.as_object().unwrap()
        );
        assert_eq!(standalone_schema["requests"]["status"], expected_target);
    }
}
