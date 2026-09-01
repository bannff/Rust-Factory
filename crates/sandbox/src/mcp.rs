#![allow(clippy::missing_errors_doc, clippy::needless_pass_by_value)]

use anyhow::{Result, anyhow};
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
    tool_router: ToolRouter<Self>,
}
impl<S, T, P> SandboxMcp<S, T, P>
where
    S: Sandbox + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    #[must_use]
    pub fn new(sandbox: S, context: T, policy: P) -> Self {
        Self {
            sandbox,
            context,
            policy,
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
        tool_response(check_size(&input).and_then(|()| {
            serialize(&json!({
                "operations": ["start", "execute", "status", "stop"],
                "durable": false, "retries": false, "recovery": false,
            }))
        }))
    }
    #[tool(
        name = "sandbox_schema",
        description = "Return generated closed request and response schemas."
    )]
    async fn schema(&self, Parameters(input): Parameters<EmptyInput>) -> String {
        tool_response(check_size(&input).and_then(|()| serialize(&json!({
            "requests": {"start": schema_for!(StartInput), "execute": schema_for!(ExecuteInput), "status": schema_for!(TargetInput), "stop": schema_for!(TargetInput)},
            "responses": {"start": schema_for!(StartOutput), "execute": schema_for!(ExecuteOutput), "status": schema_for!(StatusOutput), "stop": schema_for!(StopOutput)},
        }))))
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
}
