//! Bounded MCP control-plane adapter for workflow operations.
//!
//! Enabled by the `mcp` feature. Owns transport DTOs, generated schemas, the
//! policy gate, and safe response projection — never process lifecycle.

#![allow(unknown_lints)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::module_name_repetitions)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::unused_async_trait_impl)]

use crate::{
    AgentInvoker, AgentStep, InvocationPolicy, LogicalId, MAX_JSON_INPUT_BYTES, MAX_RUN_KEY_BYTES,
    PublicErrorCode, RequestContext, RunSummary, TerminalReason, WorkflowBudget,
    WorkflowDefinitionCatalog, WorkflowDefinitionV1, WorkflowError, WorkflowRunner, WorkflowStore,
    WorkflowVersion, canonical_json, validate_definition,
};
use agent::{AgentId, EffectiveCapabilityCeilingV1, validate_effective_capability_ceiling};
use anyhow::{Context, Result};
use policy::{
    AuthorizationDecisionV1, AuthorizationRequestV1, CapabilityV1, PolicyResolver,
    TrustedContextV1, canonical_grant, decision_digest,
};
use rmcp::{
    ServerHandler,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;

pub const WORKFLOW_TOOLS: [&str; 5] = [
    "workflow_validate",
    "workflow_start",
    "workflow_get",
    "workflow_list",
    "workflow_cancel",
];
pub const MAX_MCP_SERIALIZED_RESULT_BYTES: usize = 65_536;

/// Host-owned boundary that derives trusted request context independently of MCP input.
pub trait TrustedContextSource: Send + Sync {
    fn resolve(&self) -> Result<TrustedContextV1, WorkflowError>;
}

#[derive(Clone, Debug)]
struct AuthorizedWorkflowContext {
    context: RequestContext,
    effective_capability_ceiling: EffectiveCapabilityCeilingV1,
    policy_decision_digest: String,
}

/// Joins host-derived trusted identity with closed policy decisions.
pub struct WorkflowPolicyContextResolver<T, P> {
    source: T,
    policy: P,
}
impl<T, P> WorkflowPolicyContextResolver<T, P>
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
    ) -> Result<AuthorizedWorkflowContext, WorkflowError> {
        let trusted = self
            .source
            .resolve()
            .map_err(|_| WorkflowError::AdapterFailure)?;
        let request = AuthorizationRequestV1 {
            context: trusted.clone(),
            capability,
        };
        let decision = self.policy.authorize(request.clone());
        let AuthorizationDecisionV1::Allow {
            effective_grant,
            decision_digest: supplied_digest,
        } = decision
        else {
            return Err(WorkflowError::NotFound);
        };
        let canonical_grant =
            canonical_grant(&effective_grant).map_err(|_| WorkflowError::AdapterFailure)?;
        let expected_digest = decision_digest(
            &request,
            &AuthorizationDecisionV1::Allow {
                effective_grant: canonical_grant.clone(),
                decision_digest: String::new(),
            },
        )
        .map_err(|_| WorkflowError::AdapterFailure)?;
        if supplied_digest != expected_digest {
            return Err(WorkflowError::AdapterFailure);
        }
        Ok(AuthorizedWorkflowContext {
            context: request_context(trusted)?,
            effective_capability_ceiling: EffectiveCapabilityCeilingV1 {
                allowed_tool_ids: canonical_grant.allowed_tool_ids,
                memory_enabled: canonical_grant.memory_enabled,
                knowledge_enabled: canonical_grant.knowledge_enabled,
                sandbox_execution_allowed: canonical_grant.sandbox_execution_allowed,
                communication_allowed: canonical_grant.communication_allowed,
            },
            policy_decision_digest: supplied_digest,
        })
    }
}
fn request_context(context: TrustedContextV1) -> Result<RequestContext, WorkflowError> {
    Ok(RequestContext {
        tenant_id: LogicalId::new(context.tenant_id.as_str())?,
        principal_id: LogicalId::new(context.principal_id.as_str())?,
        request_id: LogicalId::new(context.request_id.as_str())?,
        correlation_id: LogicalId::new(context.correlation_id.as_str())?,
    })
}
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct WorkflowDefinitionInput {
    pub id: String,
    pub agent_id: String,
    pub max_input_bytes: usize,
    pub max_evidence_bytes: usize,
}
impl WorkflowDefinitionInput {
    fn into_core(self) -> Result<WorkflowDefinitionV1, WorkflowError> {
        Ok(WorkflowDefinitionV1 {
            id: LogicalId::new(self.id)?,
            version: WorkflowVersion::V1,
            step: AgentStep {
                agent_id: AgentId::new(self.agent_id)
                    .map_err(|_| WorkflowError::InvalidDefinition)?,
            },
            budget: WorkflowBudget {
                max_attempts: 1,
                max_input_bytes: self.max_input_bytes,
                max_evidence_bytes: self.max_evidence_bytes,
            },
        })
    }
}
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StartInput {
    pub workflow_id: String,
    pub run_key: String,
    pub input: String,
}
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct RunIdInput {
    pub run_id: String,
}

/// Agent invocation forwarded by the policy-aware workflow adapter.
#[derive(Clone, Debug)]
pub struct CeilingAgentInvocation {
    pub context: agent::InvocationContextV1,
    pub agent_id: AgentId,
    pub input: String,
    pub effective_capability_ceiling: EffectiveCapabilityCeilingV1,
}

fn agent_invocation_context(
    context: &RequestContext,
) -> Result<agent::InvocationContextV1, WorkflowError> {
    Ok(agent::InvocationContextV1::new(
        agent::TenantId::new(context.tenant_id.as_str())
            .map_err(|_| WorkflowError::InvalidRequest)?,
        agent::PrincipalId::new(context.principal_id.as_str())
            .map_err(|_| WorkflowError::InvalidRequest)?,
        agent::RequestId::new(context.request_id.as_str())
            .map_err(|_| WorkflowError::InvalidRequest)?,
        agent::CorrelationId::new(context.correlation_id.as_str())
            .map_err(|_| WorkflowError::InvalidRequest)?,
    ))
}

pub type CeilingInvocationFuture<'a> = std::pin::Pin<
    Box<
        dyn std::future::Future<Output = Result<agent::InvocationResult, WorkflowError>>
            + Send
            + 'a,
    >,
>;

/// Agent runtime boundary used by the workflow compatibility adapter.
pub trait CeilingAgentRuntime: Send + Sync {
    fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError>;
    fn invoke_with_ceiling<'a>(
        &'a self,
        invocation: CeilingAgentInvocation,
        control: llm_gateway::InvocationControl<'a>,
    ) -> CeilingInvocationFuture<'a>;
}

/// Internal workflow invoker for attempt policy evidence verified by `WorkflowPolicyContextResolver`.
struct PolicyAwareAgentInvoker<R> {
    runtime: R,
}
impl<R> PolicyAwareAgentInvoker<R> {
    #[must_use]
    fn new(runtime: R) -> Self {
        Self { runtime }
    }
}
impl<R: CeilingAgentRuntime> AgentInvoker for PolicyAwareAgentInvoker<R> {
    fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError> {
        self.runtime.validate_agent(id)
    }

    fn invoke<'a>(
        &'a self,
        request: crate::AgentInvocationRequest,
        control: llm_gateway::InvocationControl<'a>,
        evidence: &'a mut dyn crate::InvocationEvidenceSink,
    ) -> crate::AgentInvocationFuture<'a> {
        Box::pin(async move {
            if request.policy_decision_digest.len() != 64
                || !request
                    .policy_decision_digest
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit())
                || validate_effective_capability_ceiling(&request.effective_capability_ceiling)
                    .is_err()
            {
                return Err(WorkflowError::InvalidRequest);
            }
            let context = agent_invocation_context(&request.context)?;
            let result = self
                .runtime
                .invoke_with_ceiling(
                    CeilingAgentInvocation {
                        context,
                        agent_id: request.agent_id,
                        input: request.input,
                        effective_capability_ceiling: request.effective_capability_ceiling,
                    },
                    control,
                )
                .await?;
            evidence.emit(crate::InvocationEvidence::new(
                "llm_generation",
                canonical_model_evidence(&result.model_evidence)?,
            )?)?;
            evidence.emit(crate::InvocationEvidence::new("result", result.output)?)?;
            Ok(crate::AgentInvocationResult {
                capability_scope_digest: result.capability_scope_digest,
            })
        })
    }
}

fn canonical_model_evidence(
    evidence: &agent::InvocationModelEvidence,
) -> Result<String, WorkflowError> {
    let finish_reason = match evidence.finish_reason {
        agent::InvocationModelFinishReason::Stop => "stop",
        agent::InvocationModelFinishReason::Length => "length",
        agent::InvocationModelFinishReason::ToolCalls => "tool_calls",
        agent::InvocationModelFinishReason::ContentFilter => "content_filter",
        agent::InvocationModelFinishReason::Other => "other",
    };
    let idempotency = match evidence.idempotency {
        agent::InvocationModelIdempotency::Unsupported => "unsupported",
        agent::InvocationModelIdempotency::Accepted => "accepted",
    };
    let provider_id =
        serde_json::to_string(&evidence.provider_id).map_err(|_| WorkflowError::AdapterFailure)?;
    let model_id =
        serde_json::to_string(&evidence.model_id).map_err(|_| WorkflowError::AdapterFailure)?;
    let provider_request_id = evidence.provider_request_id.as_ref().map_or_else(
        || Ok("null".to_owned()),
        |value| serde_json::to_string(value).map_err(|_| WorkflowError::AdapterFailure),
    )?;
    let token_usage = evidence.token_usage.map_or_else(
        || "null".to_owned(),
        |usage| {
            format!(
                "{{\"input_tokens\":{},\"output_tokens\":{},\"total_tokens\":{}}}",
                usage.input_tokens, usage.output_tokens, usage.total_tokens
            )
        },
    );
    Ok(format!(
        "{{\"finish_reason\":\"{finish_reason}\",\"idempotency\":\"{idempotency}\",\"model_id\":{model_id},\"provider_id\":{provider_id},\"provider_request_id\":{provider_request_id},\"token_usage\":{token_usage}}}"
    ))
}

pub struct WorkflowMcp<S, C, R, T, P>
where
    S: WorkflowStore,
    C: WorkflowDefinitionCatalog,
    R: CeilingAgentRuntime,
    T: TrustedContextSource,
    P: PolicyResolver,
{
    runner: WorkflowRunner<S, C, PolicyAwareAgentInvoker<R>>,
    resolver: WorkflowPolicyContextResolver<T, P>,
    tool_router: ToolRouter<Self>,
}
impl<S, C, R, T, P> WorkflowMcp<S, C, R, T, P>
where
    S: WorkflowStore + 'static,
    C: WorkflowDefinitionCatalog + 'static,
    R: CeilingAgentRuntime + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    #[must_use]
    pub fn new(
        store: S,
        catalog: C,
        runtime: R,
        resolver: WorkflowPolicyContextResolver<T, P>,
        deadline_factory: Box<dyn llm_gateway::DeadlineFactory>,
        cancellation_factory: Box<dyn llm_gateway::CancellationSignalFactory>,
    ) -> Self {
        Self {
            runner: WorkflowRunner::new(
                store,
                catalog,
                PolicyAwareAgentInvoker::new(runtime),
                deadline_factory,
                cancellation_factory,
            ),
            resolver,
            tool_router: Self::tool_router(),
        }
    }
    fn validate_json(&self, input: WorkflowDefinitionInput) -> Result<String> {
        let result = (|| {
            let definition = input.into_core()?;
            validate_definition(&definition)?;
            let _authorized = self
                .resolver
                .resolve_and_authorize(CapabilityV1::WorkflowValidate)?;
            self.runner.validate(&definition)
        })();
        match result {
            Ok(()) => serialize(json!({"valid":true,"findings":[]})),
            Err(error) => {
                serialize(json!({"valid":false,"error":public_code(error.public_code())}))
            }
        }
    }
    async fn start_json(&self, input: StartInput) -> Result<String> {
        let workflow_id = LogicalId::new(input.workflow_id).map_err(public_error)?;
        if input.run_key.is_empty() || input.run_key.len() > MAX_RUN_KEY_BYTES {
            return Err(public_error(WorkflowError::InvalidRequest));
        }
        let canonical_input =
            canonical_json(&input.input, MAX_JSON_INPUT_BYTES).map_err(public_error)?;
        let authorized = self
            .resolver
            .resolve_and_authorize(CapabilityV1::WorkflowStart)
            .map_err(public_error)?;
        let summary = self
            .runner
            .start_with_policy(
                authorized.context,
                workflow_id,
                WorkflowVersion::V1,
                input.run_key,
                canonical_input,
                InvocationPolicy {
                    effective_capability_ceiling: authorized.effective_capability_ceiling,
                    policy_decision_digest: authorized.policy_decision_digest,
                },
            )
            .await
            .map_err(public_error)?;
        summary_json(&summary)
    }
    fn get_json(&self, input: RunIdInput) -> Result<String> {
        let run_id = LogicalId::new(input.run_id).map_err(public_error)?;
        let authorized = self
            .resolver
            .resolve_and_authorize(CapabilityV1::WorkflowGet)
            .map_err(public_error)?;
        let summary = self
            .runner
            .get(&authorized.context.tenant_id, run_id)
            .map_err(public_error)?;
        summary_json(&summary)
    }
    fn list_json(&self) -> Result<String> {
        let authorized = self
            .resolver
            .resolve_and_authorize(CapabilityV1::WorkflowList)
            .map_err(public_error)?;
        let runs = self
            .runner
            .list(&authorized.context.tenant_id)
            .map_err(public_error)?;
        serialize(json!({"runs":runs.iter().map(summary_value).collect::<Vec<_>>() }))
    }
    fn cancel_json(&self, input: RunIdInput) -> Result<String> {
        let run_id = LogicalId::new(input.run_id).map_err(public_error)?;
        let authorized = self
            .resolver
            .resolve_and_authorize(CapabilityV1::WorkflowCancel)
            .map_err(public_error)?;
        let summary = self
            .runner
            .cancel(&authorized.context.tenant_id, run_id)
            .map_err(public_error)?;
        summary_json(&summary)
    }
}
#[tool_router(router = tool_router)]
impl<S, C, R, T, P> WorkflowMcp<S, C, R, T, P>
where
    S: WorkflowStore + 'static,
    C: WorkflowDefinitionCatalog + 'static,
    R: CeilingAgentRuntime + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    #[tool(
        name = "workflow_validate",
        description = "Validate a bounded version-one single-agent workflow definition."
    )]
    async fn workflow_validate(
        &self,
        Parameters(input): Parameters<WorkflowDefinitionInput>,
    ) -> String {
        tool_response(self.validate_json(input))
    }
    #[tool(
        name = "workflow_start",
        description = "Start or safely replay one tenant-scoped workflow run."
    )]
    async fn workflow_start(&self, Parameters(input): Parameters<StartInput>) -> String {
        tool_response(self.start_json(input).await)
    }
    #[tool(
        name = "workflow_get",
        description = "Get one tenant-scoped workflow run summary."
    )]
    async fn workflow_get(&self, Parameters(input): Parameters<RunIdInput>) -> String {
        tool_response(self.get_json(input))
    }
    #[tool(
        name = "workflow_list",
        description = "List tenant-scoped workflow run summaries."
    )]
    async fn workflow_list(&self) -> String {
        tool_response(self.list_json())
    }
    #[tool(
        name = "workflow_cancel",
        description = "Cancel an active tenant-scoped workflow run."
    )]
    async fn workflow_cancel(&self, Parameters(input): Parameters<RunIdInput>) -> String {
        tool_response(self.cancel_json(input))
    }
}
#[tool_handler(router = self.tool_router)]
impl<S, C, R, T, P> ServerHandler for WorkflowMcp<S, C, R, T, P>
where
    S: WorkflowStore + 'static,
    C: WorkflowDefinitionCatalog + 'static,
    R: CeilingAgentRuntime + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
}
fn summary_value(summary: &RunSummary) -> serde_json::Value {
    json!({"id":summary.id.as_str(),"workflow_id":summary.workflow_id.as_str(),"workflow_version":summary.workflow_version.as_str(),"status":status_name(summary.status),"terminal_reason":summary.terminal_reason.map(reason_name),"revision":summary.revision})
}
fn summary_json(summary: &RunSummary) -> Result<String> {
    serialize(summary_value(summary))
}
fn status_name(status: crate::RunStatus) -> &'static str {
    match status {
        crate::RunStatus::Pending => "pending",
        crate::RunStatus::Running => "running",
        crate::RunStatus::Succeeded => "succeeded",
        crate::RunStatus::Failed => "failed",
        crate::RunStatus::Cancelled => "cancelled",
    }
}
fn reason_name(reason: TerminalReason) -> &'static str {
    match reason {
        TerminalReason::Completed => "completed",
        TerminalReason::InvocationFailed => "invocation_failed",
        TerminalReason::Cancelled => "cancelled",
    }
}
fn serialize(value: serde_json::Value) -> Result<String> {
    let value = serde_json::to_string(&value).context("could not serialize MCP response")?;
    (value.len() <= MAX_MCP_SERIALIZED_RESULT_BYTES)
        .then_some(value)
        .ok_or_else(|| anyhow::anyhow!("limit_exceeded"))
}
fn public_error(error: WorkflowError) -> anyhow::Error {
    anyhow::anyhow!(public_code(error.public_code()))
}
fn tool_response(response: Result<String>) -> String {
    response.unwrap_or_else(|error| {
        let code = error.to_string();
        json!({"error":if is_public(&code) { code.as_str() } else { "operation_failed" }})
            .to_string()
    })
}
fn is_public(value: &str) -> bool {
    matches!(
        value,
        "invalid_request"
            | "invalid_definition"
            | "not_found"
            | "run_key_conflict"
            | "conflict"
            | "limit_exceeded"
            | "operation_failed"
    )
}
const fn public_code(code: PublicErrorCode) -> &'static str {
    match code {
        PublicErrorCode::InvalidRequest => "invalid_request",
        PublicErrorCode::InvalidDefinition => "invalid_definition",
        PublicErrorCode::NotFound => "not_found",
        PublicErrorCode::RunKeyConflict => "run_key_conflict",
        PublicErrorCode::Conflict => "conflict",
        PublicErrorCode::LimitExceeded => "limit_exceeded",
        PublicErrorCode::OperationFailed => "operation_failed",
    }
}
#[must_use]
pub const fn tool_names() -> [&'static str; 5] {
    WORKFLOW_TOOLS
}

#[cfg(test)]
mod migration_tests {
    use super::*;
    use crate::qa_tests::TestCancellationHandle;
    use crate::{
        AgentInvocationRequest, InvocationEvidence, InvocationEvidenceSink, RequestContext,
    };
    use agent::{
        InvocationModelEvidence, InvocationModelFinishReason, InvocationModelIdempotency,
        InvocationModelTokenUsage,
    };
    use std::future::Future;
    use std::pin::Pin;
    use std::sync::{Arc, Mutex};
    use std::task::{Context, Poll, Waker};
    use std::time::{Duration, Instant};

    #[derive(Default)]
    struct Sink(Vec<InvocationEvidence>);
    impl InvocationEvidenceSink for Sink {
        fn emit(&mut self, evidence: InvocationEvidence) -> std::result::Result<(), WorkflowError> {
            self.0.push(evidence);
            Ok(())
        }
    }
    struct Deadline(Instant);
    impl llm_gateway::DeadlineSignal for Deadline {
        fn instant(&self) -> Instant {
            self.0
        }
        fn is_elapsed(&self) -> bool {
            false
        }
        fn elapsed(&self) -> llm_gateway::DeadlineFuture<'_> {
            Box::pin(std::future::pending())
        }
    }
    fn ready<T>(mut future: Pin<Box<dyn Future<Output = T> + Send + '_>>) -> T {
        match future
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
        {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("fixture unexpectedly pending"),
        }
    }
    fn evidence() -> InvocationModelEvidence {
        InvocationModelEvidence {
            provider_id: "provider".to_owned(),
            model_id: "model".to_owned(),
            provider_request_id: Some("request".to_owned()),
            finish_reason: InvocationModelFinishReason::ToolCalls,
            token_usage: Some(InvocationModelTokenUsage {
                input_tokens: 2,
                output_tokens: 3,
                total_tokens: 5,
            }),
            idempotency: InvocationModelIdempotency::Accepted,
        }
    }

    #[test]
    fn model_evidence_encoding_is_compact_closed_and_lexicographic() {
        assert_eq!(
            canonical_model_evidence(&evidence()).expect("evidence"),
            r#"{"finish_reason":"tool_calls","idempotency":"accepted","model_id":"model","provider_id":"provider","provider_request_id":"request","token_usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}}"#
        );
        let mut absent = evidence();
        absent.provider_request_id = None;
        absent.token_usage = None;
        assert!(
            canonical_model_evidence(&absent)
                .expect("evidence")
                .contains(r#""provider_request_id":null,"token_usage":null"#)
        );
    }

    struct Runtime {
        controls: Arc<Mutex<Vec<(String, Instant, bool)>>>,
    }
    impl CeilingAgentRuntime for Runtime {
        fn validate_agent(&self, _: &AgentId) -> std::result::Result<bool, WorkflowError> {
            Ok(true)
        }
        fn invoke_with_ceiling<'a>(
            &'a self,
            _: CeilingAgentInvocation,
            control: llm_gateway::InvocationControl<'a>,
        ) -> CeilingInvocationFuture<'a> {
            self.controls.lock().expect("controls").push((
                control.idempotency_key.as_str().to_owned(),
                control.deadline.instant(),
                control.cancellation.is_cancelled(),
            ));
            Box::pin(async move {
                Ok(agent::InvocationResult {
                    capability_scope_digest: "scope".to_owned(),
                    events: vec![],
                    output: "output".to_owned(),
                    model_evidence: evidence(),
                })
            })
        }
    }

    #[test]
    fn policy_invoker_forwards_control_and_emits_model_before_result() {
        let controls = Arc::new(Mutex::new(Vec::new()));
        let invoker = PolicyAwareAgentInvoker::new(Runtime {
            controls: Arc::clone(&controls),
        });
        let key = llm_gateway::IdempotencyKey::new("key").expect("key");
        let cancellation = TestCancellationHandle::default();
        let deadline = Deadline(Instant::now() + Duration::from_secs(1));
        let control = llm_gateway::InvocationControl {
            idempotency_key: &key,
            cancellation: &cancellation,
            deadline: &deadline,
        };
        let mut sink = Sink::default();
        let result = ready(invoker.invoke(
            AgentInvocationRequest {
                context: RequestContext {
                    tenant_id: LogicalId::new("tenant").expect("tenant"),
                    principal_id: LogicalId::new("principal").expect("principal"),
                    request_id: LogicalId::new("request").expect("request"),
                    correlation_id: LogicalId::new("correlation").expect("correlation"),
                },
                agent_id: AgentId::new("agent").expect("agent"),
                input: "input".to_owned(),
                attempt_id: LogicalId::new("attempt").expect("attempt"),
                effective_capability_ceiling: EffectiveCapabilityCeilingV1 {
                    allowed_tool_ids: vec![],
                    memory_enabled: false,
                    knowledge_enabled: false,
                    sandbox_execution_allowed: false,
                    communication_allowed: false,
                },
                policy_decision_digest: "a".repeat(64),
            },
            control,
            &mut sink,
        ))
        .expect("invoke");
        assert_eq!(result.capability_scope_digest, "scope");
        assert_eq!(
            sink.0
                .iter()
                .map(|item| item.kind.as_str())
                .collect::<Vec<_>>(),
            ["llm_generation", "result"]
        );
        assert_eq!(controls.lock().expect("controls")[0].0, "key");
        assert_eq!(controls.lock().expect("controls")[0].1, deadline.0);
        assert!(!controls.lock().expect("controls")[0].2);
    }
}

#[cfg(test)]
#[path = "mcp/qa_tests.rs"]
mod qa_tests;
