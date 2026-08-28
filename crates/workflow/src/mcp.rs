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
    pub agent_id: AgentId,
    pub input: String,
    pub effective_capability_ceiling: EffectiveCapabilityCeilingV1,
    pub cancellation: crate::CancellationSignal,
    pub deadline: std::time::Instant,
    pub downstream_idempotency_key: String,
}

/// Agent runtime boundary used by the workflow compatibility adapter.
pub trait CeilingAgentRuntime: Send + Sync {
    fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError>;
    fn invoke_with_ceiling(
        &self,
        invocation: CeilingAgentInvocation,
    ) -> Result<agent::InvocationResult, WorkflowError>;
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

    fn invoke(
        &self,
        request: crate::AgentInvocationRequest,
        evidence: &mut dyn crate::InvocationEvidenceSink,
    ) -> Result<crate::AgentInvocationResult, WorkflowError> {
        if request.policy_decision_digest.len() != 64
            || !request
                .policy_decision_digest
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || validate_effective_capability_ceiling(&request.effective_capability_ceiling).is_err()
        {
            return Err(WorkflowError::InvalidRequest);
        }
        let result = self.runtime.invoke_with_ceiling(CeilingAgentInvocation {
            agent_id: request.agent_id,
            input: request.input,
            effective_capability_ceiling: request.effective_capability_ceiling,
            cancellation: request.cancellation,
            deadline: request.deadline,
            downstream_idempotency_key: request.downstream_idempotency_key,
        })?;
        evidence.emit(crate::InvocationEvidence::new("result", result.output)?)?;
        Ok(crate::AgentInvocationResult {
            capability_scope_digest: result.capability_scope_digest,
        })
    }
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
    ) -> Self {
        Self {
            runner: WorkflowRunner::new(store, catalog, PolicyAwareAgentInvoker::new(runtime)),
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
    fn start_json(&self, input: StartInput) -> Result<String> {
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
        tool_response(self.start_json(input))
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
mod tests {
    use super::*;

    #[test]
    fn exposes_only_workflow_tools_and_start_has_no_caller_identity_fields() {
        assert_eq!(
            tool_names(),
            [
                "workflow_validate",
                "workflow_start",
                "workflow_get",
                "workflow_list",
                "workflow_cancel"
            ]
        );
        assert!(
            serde_json::from_value::<StartInput>(
                json!({"workflow_id":"workflow","run_key":"key","input":"{}","tenant_id":"forged"})
            )
            .is_err()
        );
        assert!(
            serde_json::from_value::<RunIdInput>(json!({"run_id":"run","principal_id":"forged"}))
                .is_err()
        );
    }

    #[test]
    fn public_error_responses_do_not_expose_adapter_details() {
        assert_eq!(
            tool_response(Err(anyhow::anyhow!("credential=secret"))),
            "{\"error\":\"operation_failed\"}"
        );
        assert_eq!(
            tool_response(Err(anyhow::anyhow!("not_found"))),
            "{\"error\":\"not_found\"}"
        );
    }
}

#[cfg(test)]
mod compatibility_tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    use crate::{
        AgentInvocationRequest, CancellationSignal, InvocationEvidence, InvocationEvidenceSink,
        Transition, TransitionResult,
    };
    use policy::{
        CorrelationId, GrantV1, PrincipalId, RequestId, TenantId, allow_decision, deny_decision,
    };

    #[derive(Clone, Default)]
    struct RecordingDomain(Arc<Mutex<Vec<&'static str>>>);
    impl RecordingDomain {
        fn record(&self, operation: &'static str) {
            self.0.lock().expect("domain calls lock").push(operation);
        }
        fn calls(&self) -> Vec<&'static str> {
            self.0.lock().expect("domain calls lock").clone()
        }
    }
    impl WorkflowStore for RecordingDomain {
        fn create_or_return(
            &self,
            _: crate::StartIdentity,
            _: crate::Run,
        ) -> std::result::Result<crate::CreateRun, WorkflowError> {
            self.record("store.create");
            Err(WorkflowError::AdapterFailure)
        }
        fn get(
            &self,
            _: &LogicalId,
            _: &LogicalId,
        ) -> std::result::Result<Option<crate::Run>, WorkflowError> {
            self.record("store.get");
            Err(WorkflowError::AdapterFailure)
        }
        fn list(&self, _: &LogicalId) -> std::result::Result<Vec<crate::Run>, WorkflowError> {
            self.record("store.list");
            Err(WorkflowError::AdapterFailure)
        }
        fn transition(
            &self,
            _: &LogicalId,
            _: &LogicalId,
            _: u64,
            _: crate::RunStatus,
            _: Transition,
        ) -> std::result::Result<TransitionResult, WorkflowError> {
            self.record("store.transition");
            Err(WorkflowError::AdapterFailure)
        }
    }
    impl WorkflowDefinitionCatalog for RecordingDomain {
        fn resolve(
            &self,
            _: &LogicalId,
            _: WorkflowVersion,
        ) -> std::result::Result<Option<WorkflowDefinitionV1>, WorkflowError> {
            self.record("catalog.resolve");
            Err(WorkflowError::AdapterFailure)
        }
    }
    impl CeilingAgentRuntime for RecordingDomain {
        fn validate_agent(&self, _: &AgentId) -> std::result::Result<bool, WorkflowError> {
            self.record("invoker.validate");
            Err(WorkflowError::AdapterFailure)
        }
        fn invoke_with_ceiling(
            &self,
            _: CeilingAgentInvocation,
        ) -> std::result::Result<agent::InvocationResult, WorkflowError> {
            self.record("invoker.invoke");
            Err(WorkflowError::AdapterFailure)
        }
    }

    #[derive(Clone)]
    struct RecordingSource {
        context: std::result::Result<TrustedContextV1, WorkflowError>,
    }
    impl TrustedContextSource for RecordingSource {
        fn resolve(&self) -> std::result::Result<TrustedContextV1, WorkflowError> {
            self.context.clone()
        }
    }
    #[derive(Clone)]
    struct RecordingPolicy {
        allow: bool,
        calls: Arc<Mutex<Vec<CapabilityV1>>>,
    }
    impl PolicyResolver for RecordingPolicy {
        fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
            self.calls
                .lock()
                .expect("policy calls lock")
                .push(request.capability);
            if self.allow {
                allow_decision(
                    &request,
                    &GrantV1::new(["tool".to_owned()], true, false, false, false).expect("grant"),
                )
                .expect("allow")
            } else {
                deny_decision()
            }
        }
    }

    type RecordingMcp = WorkflowMcp<
        RecordingDomain,
        RecordingDomain,
        RecordingDomain,
        RecordingSource,
        RecordingPolicy,
    >;

    fn trusted_context() -> TrustedContextV1 {
        TrustedContextV1 {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            principal_id: PrincipalId::new("principal").expect("principal"),
            request_id: RequestId::new("request").expect("request"),
            correlation_id: CorrelationId::new("correlation").expect("correlation"),
        }
    }
    fn service(
        source: std::result::Result<TrustedContextV1, WorkflowError>,
        allow: bool,
    ) -> (RecordingMcp, RecordingDomain, Arc<Mutex<Vec<CapabilityV1>>>) {
        let domain = RecordingDomain::default();
        let policy_calls = Arc::new(Mutex::new(Vec::new()));
        let resolver = WorkflowPolicyContextResolver::new(
            RecordingSource { context: source },
            RecordingPolicy {
                allow,
                calls: Arc::clone(&policy_calls),
            },
        );
        (
            WorkflowMcp::new(domain.clone(), domain.clone(), domain.clone(), resolver),
            domain,
            policy_calls,
        )
    }

    #[derive(Clone, Copy)]
    enum Operation {
        Validate,
        Start,
        Get,
        List,
        Cancel,
    }
    impl Operation {
        const fn capability(self) -> CapabilityV1 {
            match self {
                Self::Validate => CapabilityV1::WorkflowValidate,
                Self::Start => CapabilityV1::WorkflowStart,
                Self::Get => CapabilityV1::WorkflowGet,
                Self::List => CapabilityV1::WorkflowList,
                Self::Cancel => CapabilityV1::WorkflowCancel,
            }
        }
    }
    fn call(service: &RecordingMcp, operation: Operation) -> String {
        match operation {
            Operation::Validate => service
                .validate_json(WorkflowDefinitionInput {
                    id: "workflow".to_owned(),
                    agent_id: "agent".to_owned(),
                    max_input_bytes: 1,
                    max_evidence_bytes: 1,
                })
                .expect("response"),
            Operation::Start => tool_response(service.start_json(StartInput {
                workflow_id: "workflow".to_owned(),
                run_key: "key".to_owned(),
                input: "{}".to_owned(),
            })),
            Operation::Get => tool_response(service.get_json(RunIdInput {
                run_id: "run".to_owned(),
            })),
            Operation::List => tool_response(service.list_json()),
            Operation::Cancel => tool_response(service.cancel_json(RunIdInput {
                run_id: "run".to_owned(),
            })),
        }
    }

    #[test]
    fn source_failure_is_pre_domain_for_every_operation() {
        for operation in [
            Operation::Validate,
            Operation::Start,
            Operation::Get,
            Operation::List,
            Operation::Cancel,
        ] {
            let (service, domain, policy_calls) = service(Err(WorkflowError::AdapterFailure), true);
            let response = call(&service, operation);
            let expected = if matches!(operation, Operation::Validate) {
                r#"{"error":"operation_failed","valid":false}"#
            } else {
                r#"{"error":"operation_failed"}"#
            };
            assert_eq!(response, expected);
            assert!(domain.calls().is_empty());
            assert!(policy_calls.lock().expect("policy calls lock").is_empty());
        }
    }

    #[test]
    fn deny_is_pre_domain_and_uses_the_exact_operation_capability() {
        for operation in [
            Operation::Validate,
            Operation::Start,
            Operation::Get,
            Operation::List,
            Operation::Cancel,
        ] {
            let (service, domain, policy_calls) = service(Ok(trusted_context()), false);
            let response = call(&service, operation);
            let expected = if matches!(operation, Operation::Validate) {
                r#"{"error":"not_found","valid":false}"#
            } else {
                r#"{"error":"not_found"}"#
            };
            assert_eq!(response, expected);
            assert!(domain.calls().is_empty());
            assert_eq!(
                policy_calls.lock().expect("policy calls lock").as_slice(),
                &[operation.capability()]
            );
        }
    }

    #[derive(Clone, Copy)]
    struct TamperedPolicy;
    impl PolicyResolver for TamperedPolicy {
        fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
            let grant =
                GrantV1::new(["tool".to_owned()], true, false, false, false).expect("grant");
            let AuthorizationDecisionV1::Allow {
                effective_grant, ..
            } = allow_decision(&request, &grant).expect("allow")
            else {
                unreachable!("allow_decision must allow");
            };
            AuthorizationDecisionV1::Allow {
                effective_grant,
                decision_digest: "0".repeat(64),
            }
        }
    }

    #[test]
    fn tampered_allow_decision_evidence_is_pre_domain() {
        let domain = RecordingDomain::default();
        let resolver = WorkflowPolicyContextResolver::new(
            RecordingSource {
                context: Ok(trusted_context()),
            },
            TamperedPolicy,
        );
        let service = WorkflowMcp::new(domain.clone(), domain.clone(), domain.clone(), resolver);

        assert_eq!(
            tool_response(service.start_json(StartInput {
                workflow_id: "workflow".to_owned(),
                run_key: "key".to_owned(),
                input: "{}".to_owned(),
            })),
            r#"{"error":"operation_failed"}"#
        );
        assert!(domain.calls().is_empty());
    }

    #[test]
    fn invalid_validate_and_start_inputs_are_pre_policy_and_pre_domain() {
        let (service, domain, policy_calls) = service(Ok(trusted_context()), true);
        assert_eq!(
            service
                .validate_json(WorkflowDefinitionInput {
                    id: "workflow".to_owned(),
                    agent_id: "agent".to_owned(),
                    max_input_bytes: 0,
                    max_evidence_bytes: 1,
                })
                .expect("response"),
            r#"{"error":"invalid_definition","valid":false}"#
        );
        assert!(domain.calls().is_empty());
        assert!(policy_calls.lock().expect("policy calls lock").is_empty());

        for input in [
            StartInput {
                workflow_id: "Invalid".to_owned(),
                run_key: "key".to_owned(),
                input: "{}".to_owned(),
            },
            StartInput {
                workflow_id: "workflow".to_owned(),
                run_key: String::new(),
                input: "{}".to_owned(),
            },
            StartInput {
                workflow_id: "workflow".to_owned(),
                run_key: "key".to_owned(),
                input: "not-json".to_owned(),
            },
            StartInput {
                workflow_id: "workflow".to_owned(),
                run_key: "key".to_owned(),
                input: "x".repeat(MAX_JSON_INPUT_BYTES + 1),
            },
        ] {
            assert!(service.start_json(input).is_err());
            assert!(domain.calls().is_empty());
            assert!(policy_calls.lock().expect("policy calls lock").is_empty());
        }
    }

    #[test]
    fn allow_uses_the_exact_capability_before_reaching_each_domain_path() {
        for operation in [
            Operation::Validate,
            Operation::Start,
            Operation::Get,
            Operation::List,
            Operation::Cancel,
        ] {
            let (service, domain, policy_calls) = service(Ok(trusted_context()), true);
            let response = call(&service, operation);

            assert_eq!(
                policy_calls.lock().expect("policy calls lock").as_slice(),
                &[operation.capability()],
                "the allow path must authorize the operation's closed capability"
            );
            assert!(
                !domain.calls().is_empty(),
                "an allowed operation must preserve its existing domain path"
            );
            let expected = if matches!(operation, Operation::Validate) {
                r#"{"error":"operation_failed","valid":false}"#
            } else {
                r#"{"error":"operation_failed"}"#
            };
            assert_eq!(response, expected);
        }
    }

    #[derive(Default)]
    struct EvidenceSink(Vec<InvocationEvidence>);
    impl InvocationEvidenceSink for EvidenceSink {
        fn emit(&mut self, evidence: InvocationEvidence) -> std::result::Result<(), WorkflowError> {
            self.0.push(evidence);
            Ok(())
        }
    }
    #[derive(Clone)]
    struct RecordingRuntime {
        calls: Arc<Mutex<Vec<CeilingAgentInvocation>>>,
    }
    impl CeilingAgentRuntime for RecordingRuntime {
        fn validate_agent(&self, _: &AgentId) -> std::result::Result<bool, WorkflowError> {
            Ok(true)
        }
        fn invoke_with_ceiling(
            &self,
            invocation: CeilingAgentInvocation,
        ) -> std::result::Result<agent::InvocationResult, WorkflowError> {
            self.calls
                .lock()
                .expect("runtime calls lock")
                .push(invocation);
            Ok(agent::InvocationResult {
                capability_scope_digest: "scope".to_owned(),
                events: vec![],
                output: "output".to_owned(),
            })
        }
    }

    #[test]
    fn policy_aware_invoker_rejects_invalid_policy_evidence_before_runtime_access() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let invoker = PolicyAwareAgentInvoker::new(RecordingRuntime {
            calls: Arc::clone(&calls),
        });
        let request = AgentInvocationRequest {
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
            policy_decision_digest: "invalid".to_owned(),
            downstream_idempotency_key: "key".to_owned(),
            cancellation: CancellationSignal::new(),
            deadline: std::time::Instant::now() + std::time::Duration::from_secs(1),
        };
        let mut evidence = EvidenceSink::default();

        assert_eq!(
            invoker.invoke(request, &mut evidence),
            Err(WorkflowError::InvalidRequest),
            "invalid decision evidence must fail before the agent runtime"
        );
        assert!(calls.lock().expect("runtime calls lock").is_empty());
        assert!(evidence.0.is_empty());
    }

    #[test]
    fn policy_aware_invoker_forwards_only_the_attempt_bound_ceiling() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let invoker = PolicyAwareAgentInvoker::new(RecordingRuntime {
            calls: Arc::clone(&calls),
        });
        let ceiling = EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: vec!["tool".to_owned()],
            memory_enabled: true,
            knowledge_enabled: false,
            sandbox_execution_allowed: false,
            communication_allowed: false,
        };
        let mut evidence = EvidenceSink::default();
        let result = invoker
            .invoke(
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
                    effective_capability_ceiling: ceiling.clone(),
                    policy_decision_digest: "a".repeat(64),
                    downstream_idempotency_key: "key".to_owned(),
                    cancellation: CancellationSignal::new(),
                    deadline: std::time::Instant::now() + std::time::Duration::from_secs(1),
                },
                &mut evidence,
            )
            .expect("invoke");
        assert_eq!(result.capability_scope_digest, "scope");
        assert_eq!(
            evidence.0,
            vec![InvocationEvidence::new("result", "output").expect("result")]
        );
        let calls = calls.lock().expect("runtime calls lock");
        assert_eq!(calls.len(), 1);
        let call = &calls[0];
        assert_eq!(call.agent_id, AgentId::new("agent").expect("agent"));
        assert_eq!(call.input, "input");
        assert_eq!(call.effective_capability_ceiling, ceiling);
        assert!(!call.cancellation.is_cancelled());
        assert!(call.deadline > std::time::Instant::now());
        assert_eq!(call.downstream_idempotency_key, "key");
    }
}

#[cfg(all(test, feature = "memory"))]
mod composition_tests {
    use super::*;
    use std::sync::{Arc, Barrier, Mutex};
    use std::thread;

    use crate::memory::{InMemoryWorkflowStore, StaticWorkflowCatalog};
    use agent::{
        AgentDefinitionV1, AgentRegistry, CommunicationPolicy, DefinitionError, DefinitionVersion,
        DenySandbox, ExecutionLimits, InMemoryDefinitionStore, InMemoryMemoryStore,
        KnowledgePolicy, LocalAgentRuntime, MemoryPolicy, ModelPolicy, ModelProvider, ModelRequest,
        ModelResponse, SandboxPolicy, StaticKnowledgeStore, StaticReferenceCatalog, ToolCall,
        ToolDescriptor, ToolRegistry, ToolRequest,
    };
    use policy::{CorrelationId, GrantV1, PrincipalId, RequestId, TenantId, allow_decision};

    const AGENT_DEFINITION_TOOLS: [&str; 2] = ["read", "write"];
    const ALLOWED_TOOL: &str = AGENT_DEFINITION_TOOLS[0];
    const DENIED_DEFINITION_TOOL: &str = AGENT_DEFINITION_TOOLS[1];

    #[derive(Clone)]
    struct SharedContextSource(Arc<Mutex<TrustedContextV1>>);
    impl SharedContextSource {
        fn new(context: TrustedContextV1) -> Self {
            Self(Arc::new(Mutex::new(context)))
        }

        fn set(&self, context: TrustedContextV1) {
            *self.0.lock().expect("context lock") = context;
        }
    }
    impl TrustedContextSource for SharedContextSource {
        fn resolve(&self) -> Result<TrustedContextV1, WorkflowError> {
            Ok(self.0.lock().expect("context lock").clone())
        }
    }

    #[derive(Clone, Copy)]
    struct AllowingPolicy;
    impl PolicyResolver for AllowingPolicy {
        fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
            allow_decision(
                &request,
                &GrantV1::new([ALLOWED_TOOL.to_owned()], true, false, false, false)
                    .expect("valid grant"),
            )
            .expect("valid decision")
        }
    }

    #[derive(Clone, Default)]
    struct RecordingModelProvider {
        requests: Arc<Mutex<Vec<ModelRequest>>>,
    }
    impl RecordingModelProvider {
        fn requests(&self) -> Vec<ModelRequest> {
            self.requests.lock().expect("model requests lock").clone()
        }
    }
    impl ModelProvider for RecordingModelProvider {
        fn invoke(&self, request: ModelRequest) -> Result<ModelResponse, DefinitionError> {
            self.requests
                .lock()
                .expect("model requests lock")
                .push(request);
            Ok(ModelResponse {
                output: "completed".to_owned(),
                tool_calls: vec![ToolCall {
                    tool_id: ALLOWED_TOOL.to_owned(),
                    input: "input".to_owned(),
                }],
                capability_requests: vec![],
            })
        }
    }

    #[derive(Clone, Default)]
    struct RecordingToolRegistry {
        resolved: Arc<Mutex<Vec<String>>>,
        invoked: Arc<Mutex<Vec<String>>>,
    }
    impl RecordingToolRegistry {
        fn resolved(&self) -> Vec<String> {
            self.resolved.lock().expect("resolved tools lock").clone()
        }

        fn invoked(&self) -> Vec<String> {
            self.invoked.lock().expect("invoked tools lock").clone()
        }
    }
    impl ToolRegistry for RecordingToolRegistry {
        fn resolve(&self, id: &str) -> Result<ToolDescriptor, DefinitionError> {
            self.resolved
                .lock()
                .expect("resolved tools lock")
                .push(id.to_owned());
            Ok(ToolDescriptor { id: id.to_owned() })
        }

        fn invoke(&self, tool: &ToolDescriptor, _: ToolRequest) -> Result<String, DefinitionError> {
            self.invoked
                .lock()
                .expect("invoked tools lock")
                .push(tool.id.clone());
            Ok("tool output".to_owned())
        }
    }

    struct LocalRuntimeCeilingAdapter {
        registry: AgentRegistry<InMemoryDefinitionStore, StaticReferenceCatalog>,
        model: RecordingModelProvider,
        tools: RecordingToolRegistry,
        memory: InMemoryMemoryStore,
        knowledge: StaticKnowledgeStore,
        sandbox: DenySandbox,
    }
    impl LocalRuntimeCeilingAdapter {
        fn new(model: RecordingModelProvider, tools: RecordingToolRegistry) -> Self {
            let definition = AgentDefinitionV1 {
                version: DefinitionVersion::V1,
                id: AgentId::new("agent").expect("agent"),
                name: "Agent".to_owned(),
                description: "A deterministic test agent".to_owned(),
                model: ModelPolicy {
                    reference: "model".to_owned(),
                },
                instructions: "Use the allowed tool.".to_owned(),
                skills: vec![],
                steering: vec![],
                allowed_tool_ids: AGENT_DEFINITION_TOOLS.map(str::to_owned).to_vec(),
                memory: MemoryPolicy {
                    enabled: false,
                    max_items: 0,
                },
                knowledge: KnowledgePolicy {
                    enabled: false,
                    max_results: 0,
                },
                sandbox: SandboxPolicy {
                    allow_execution: false,
                },
                communication: CommunicationPolicy {
                    allow_messages: false,
                },
                limits: ExecutionLimits {
                    max_tool_calls: 1,
                    max_output_bytes: 1_024,
                },
            };
            let catalog = StaticReferenceCatalog::new(
                ["model".to_owned()],
                [],
                [],
                AGENT_DEFINITION_TOOLS.map(str::to_owned),
            );
            Self {
                registry: AgentRegistry::new(
                    vec![definition],
                    InMemoryDefinitionStore::default(),
                    catalog,
                )
                .expect("test registry"),
                model,
                tools,
                memory: InMemoryMemoryStore::default(),
                knowledge: StaticKnowledgeStore::default(),
                sandbox: DenySandbox,
            }
        }
    }
    impl CeilingAgentRuntime for LocalRuntimeCeilingAdapter {
        fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError> {
            self.registry
                .get(id)
                .map(|_| true)
                .map_err(|_| WorkflowError::AdapterFailure)
        }

        fn invoke_with_ceiling(
            &self,
            invocation: CeilingAgentInvocation,
        ) -> Result<agent::InvocationResult, WorkflowError> {
            LocalAgentRuntime::new(
                &self.registry,
                &self.model,
                &self.tools,
                &self.memory,
                &self.knowledge,
                &self.sandbox,
            )
            .invoke_with_ceiling(
                &invocation.agent_id,
                invocation.input,
                &invocation.effective_capability_ceiling,
            )
            .map_err(|_| WorkflowError::AdapterFailure)
        }
    }

    #[derive(Clone, Default)]
    struct RecordingCeilingRuntime {
        calls: Arc<Mutex<Vec<CeilingAgentInvocation>>>,
    }
    impl RecordingCeilingRuntime {
        fn calls(&self) -> Vec<CeilingAgentInvocation> {
            self.calls.lock().expect("runtime calls lock").clone()
        }
    }
    impl CeilingAgentRuntime for RecordingCeilingRuntime {
        fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError> {
            Ok(id.as_str() == "agent")
        }

        fn invoke_with_ceiling(
            &self,
            invocation: CeilingAgentInvocation,
        ) -> Result<agent::InvocationResult, WorkflowError> {
            self.calls
                .lock()
                .expect("runtime calls lock")
                .push(invocation);
            Ok(agent::InvocationResult {
                capability_scope_digest: "scope".to_owned(),
                events: vec![],
                output: "output".to_owned(),
            })
        }
    }

    #[derive(Clone)]
    struct BlockingCeilingRuntime {
        calls: Arc<Mutex<Vec<CeilingAgentInvocation>>>,
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
    }
    impl BlockingCeilingRuntime {
        fn new(entered: Arc<Barrier>, release: Arc<Barrier>) -> Self {
            Self {
                calls: Arc::new(Mutex::new(Vec::new())),
                entered,
                release,
            }
        }

        fn calls(&self) -> Vec<CeilingAgentInvocation> {
            self.calls.lock().expect("runtime calls lock").clone()
        }
    }
    impl CeilingAgentRuntime for BlockingCeilingRuntime {
        fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError> {
            Ok(id.as_str() == "agent")
        }

        fn invoke_with_ceiling(
            &self,
            invocation: CeilingAgentInvocation,
        ) -> Result<agent::InvocationResult, WorkflowError> {
            self.calls
                .lock()
                .expect("runtime calls lock")
                .push(invocation.clone());
            self.entered.wait();
            self.release.wait();
            if invocation.cancellation.is_cancelled() {
                return Err(WorkflowError::Conflict);
            }
            Ok(agent::InvocationResult {
                capability_scope_digest: "scope".to_owned(),
                events: vec![],
                output: "output".to_owned(),
            })
        }
    }

    fn context(tenant: &str) -> TrustedContextV1 {
        TrustedContextV1 {
            tenant_id: TenantId::new(tenant).expect("tenant"),
            principal_id: PrincipalId::new("principal").expect("principal"),
            request_id: RequestId::new("request").expect("request"),
            correlation_id: CorrelationId::new("correlation").expect("correlation"),
        }
    }

    fn catalog() -> StaticWorkflowCatalog {
        StaticWorkflowCatalog::new([WorkflowDefinitionV1 {
            id: LogicalId::new("workflow").expect("workflow"),
            version: WorkflowVersion::V1,
            step: AgentStep {
                agent_id: AgentId::new("agent").expect("agent"),
            },
            budget: WorkflowBudget::default(),
        }])
    }

    fn resolver(
        source: SharedContextSource,
    ) -> WorkflowPolicyContextResolver<SharedContextSource, AllowingPolicy> {
        WorkflowPolicyContextResolver::new(source, AllowingPolicy)
    }

    fn start_input() -> StartInput {
        StartInput {
            workflow_id: "workflow".to_owned(),
            run_key: "run-key".to_owned(),
            input: r#"{"request":"value"}"#.to_owned(),
        }
    }

    fn run_id(response: &str) -> String {
        serde_json::from_str::<serde_json::Value>(response)
            .expect("start response")
            .get("id")
            .and_then(serde_json::Value::as_str)
            .expect("run id")
            .to_owned()
    }

    fn assert_grant_ceiling(invocation: &CeilingAgentInvocation) {
        assert!(
            AGENT_DEFINITION_TOOLS.contains(&DENIED_DEFINITION_TOOL),
            "the agent definition must conceptually contain the denied tool"
        );
        assert_eq!(
            invocation.effective_capability_ceiling.allowed_tool_ids,
            [ALLOWED_TOOL]
        );
        assert!(
            !invocation
                .effective_capability_ceiling
                .allowed_tool_ids
                .iter()
                .any(|tool| tool == DENIED_DEFINITION_TOOL),
            "the policy grant must remove a tool conceptually present in the agent definition"
        );
    }

    #[test]
    fn workflow_mcp_start_composes_policy_ceiling_with_local_agent_runtime() {
        let source = SharedContextSource::new(context("tenant"));
        let model = RecordingModelProvider::default();
        let tools = RecordingToolRegistry::default();
        let service = WorkflowMcp::new(
            InMemoryWorkflowStore::default(),
            catalog(),
            LocalRuntimeCeilingAdapter::new(model.clone(), tools.clone()),
            resolver(source),
        );

        let response = service.start_json(start_input()).expect("start");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&response).expect("start response")["status"],
            "succeeded"
        );

        let requests = model.requests();
        assert_eq!(requests.len(), 1);
        assert_eq!(
            requests[0].capability_scope.allowed_tool_ids,
            [ALLOWED_TOOL]
        );
        assert!(
            !requests[0]
                .capability_scope
                .allowed_tool_ids
                .iter()
                .any(|tool| tool == DENIED_DEFINITION_TOOL),
            "the workflow grant must exclude the definition's write tool from the model scope"
        );
        assert!(
            tools.resolved().iter().all(|tool| tool == ALLOWED_TOOL),
            "write must not reach ToolRegistry::resolve"
        );
        assert_eq!(tools.invoked(), [ALLOWED_TOOL]);
    }

    #[test]
    fn workflow_mcp_start_composes_policy_ceiling_replay_and_tenant_isolation() {
        let store = InMemoryWorkflowStore::default();
        let source = SharedContextSource::new(context("tenant-a"));
        let runtime = RecordingCeilingRuntime::default();
        let service = WorkflowMcp::new(
            store.clone(),
            catalog(),
            runtime.clone(),
            resolver(source.clone()),
        );

        let first = service.start_json(start_input()).expect("start");
        let first_run_id = run_id(&first);
        let calls = runtime.calls();
        assert_eq!(calls.len(), 1);
        assert_grant_ceiling(&calls[0]);
        assert!(!calls[0].cancellation.is_cancelled());
        assert!(calls[0].deadline > std::time::Instant::now());
        assert_eq!(calls[0].downstream_idempotency_key.len(), 64);
        assert!(
            calls[0]
                .downstream_idempotency_key
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
        );

        assert_eq!(service.start_json(start_input()).expect("replay"), first);
        assert_eq!(
            runtime.calls().len(),
            1,
            "replay must not reinvoke the runtime"
        );

        source.set(context("tenant-b"));
        let other_tenant = WorkflowMcp::new(store, catalog(), runtime.clone(), resolver(source));
        assert!(
            other_tenant
                .get_json(RunIdInput {
                    run_id: first_run_id
                })
                .is_err()
        );
        assert_eq!(other_tenant.list_json().expect("list"), r#"{"runs":[]}"#);
    }

    #[test]
    fn workflow_mcp_cancellation_signals_the_grant_scoped_runtime_invocation() {
        let store = InMemoryWorkflowStore::default();
        let source = SharedContextSource::new(context("tenant"));
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let runtime = BlockingCeilingRuntime::new(Arc::clone(&entered), Arc::clone(&release));
        let service = Arc::new(WorkflowMcp::new(
            store.clone(),
            catalog(),
            runtime.clone(),
            resolver(source),
        ));
        let worker = Arc::clone(&service);
        let start = thread::spawn(move || worker.start_json(start_input()));

        entered.wait();
        let invocation = runtime.calls().pop().expect("runtime invocation");
        assert_grant_ceiling(&invocation);
        let run_id = store
            .list(&LogicalId::new("tenant").expect("tenant"))
            .expect("runs")
            .pop()
            .expect("running run")
            .id
            .as_str()
            .to_owned();
        let cancelled = service.cancel_json(RunIdInput { run_id }).expect("cancel");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&cancelled).expect("cancel response")["status"],
            "cancelled"
        );
        assert!(invocation.cancellation.is_cancelled());

        release.wait();
        let completed = start
            .join()
            .expect("worker thread")
            .expect("start response");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&completed).expect("start response")["status"],
            "cancelled"
        );
    }
}
