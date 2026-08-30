//! Bounded Policy-gated MCP adapter.
#![allow(unknown_lints)]
#![allow(
    clippy::missing_errors_doc,
    clippy::module_name_repetitions,
    clippy::needless_pass_by_value,
    clippy::unused_async_trait_impl
)]
use crate::{
    CreateOrMatch, CriterionV1, EvaluationDefinitionV1, EvaluationError, EvaluationExecutor,
    EvaluationService, EvaluationStore, LogicalEvaluationKey, WorkflowEvidenceReader,
    definition_digest, result_canonical_bytes, validate_definition, validate_logical_key,
};
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

pub const EVALUATION_TOOLS: [&str; 3] = [
    "evaluation_validate",
    "evaluation_evaluate_run",
    "evaluation_get_result",
];
/// Maximum serialized size of an Evaluation tool's parameter DTO.
///
/// This is not a full MCP or JSON-RPC envelope limit. The composition transport
/// must reject oversized envelopes before buffering or deserializing tool input.
pub const MAX_MCP_REQUEST_DTO_BYTES: usize = 65_536;
pub const MAX_MCP_SERIALIZED_RESULT_BYTES: usize = 65_536;
pub const MAX_MCP_TOOL_TEXT_BYTES: usize = 56 * 1024;
pub const MAX_MCP_RESULT_BYTES: usize = 28 * 1024;
pub trait TrustedContextSource: Send + Sync {
    fn resolve(&self) -> Result<TrustedContextV1, EvaluationError>;
}
pub struct EvaluationPolicyContextResolver<T, P> {
    source: T,
    policy: P,
}
impl<T: TrustedContextSource, P: PolicyResolver> EvaluationPolicyContextResolver<T, P> {
    #[must_use]
    pub const fn new(source: T, policy: P) -> Self {
        Self { source, policy }
    }
    fn resolve_and_authorize(&self, capability: CapabilityV1) -> Result<String, EvaluationError> {
        let trusted = self
            .source
            .resolve()
            .map_err(|_| EvaluationError::AdapterFailure)?;
        let request = AuthorizationRequestV1 {
            context: trusted.clone(),
            capability,
        };
        let AuthorizationDecisionV1::Allow {
            effective_grant,
            decision_digest: supplied,
        } = self.policy.authorize(request.clone())
        else {
            return Err(EvaluationError::NotFound);
        };
        let effective_grant =
            canonical_grant(&effective_grant).map_err(|_| EvaluationError::AdapterFailure)?;
        let expected = decision_digest(
            &request,
            &AuthorizationDecisionV1::Allow {
                effective_grant,
                decision_digest: String::new(),
            },
        )
        .map_err(|_| EvaluationError::AdapterFailure)?;
        if supplied != expected {
            return Err(EvaluationError::AdapterFailure);
        }
        Ok(trusted.tenant_id.as_str().to_owned())
    }
}
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CriterionInput {
    ExactOutput { expected: String },
    EventKindCount { kind: String, expected: u32 },
    EventDataEquals { sequence: u64, expected: String },
}
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct DefinitionInput {
    pub evaluator_id: String,
    pub evaluator_version: String,
    pub criteria: Vec<CriterionInput>,
}
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluateRunInput {
    pub run_id: String,
    #[serde(flatten)]
    pub definition: DefinitionInput,
}
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct GetResultInput {
    pub evaluator_id: String,
    pub evaluator_version: String,
    pub criterion_digest: String,
    pub run_id: String,
    pub workflow_revision: u64,
}
impl DefinitionInput {
    fn into_core(self) -> EvaluationDefinitionV1 {
        EvaluationDefinitionV1 {
            evaluator_id: self.evaluator_id,
            evaluator_version: self.evaluator_version,
            criteria: self
                .criteria
                .into_iter()
                .map(|criterion| match criterion {
                    CriterionInput::ExactOutput { expected } => {
                        CriterionV1::ExactOutput { expected }
                    }
                    CriterionInput::EventKindCount { kind, expected } => {
                        CriterionV1::EventKindCount { kind, expected }
                    }
                    CriterionInput::EventDataEquals { sequence, expected } => {
                        CriterionV1::EventDataEquals { sequence, expected }
                    }
                })
                .collect(),
        }
    }
}
pub struct EvaluationMcp<R, S, E, T, P>
where
    R: WorkflowEvidenceReader,
    S: EvaluationStore,
    E: EvaluationExecutor,
    T: TrustedContextSource,
    P: PolicyResolver,
{
    service: EvaluationService<R, S, E>,
    resolver: EvaluationPolicyContextResolver<T, P>,
    tool_router: ToolRouter<Self>,
}
impl<R, S, E, T, P> EvaluationMcp<R, S, E, T, P>
where
    R: WorkflowEvidenceReader + 'static,
    S: EvaluationStore + 'static,
    E: EvaluationExecutor + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    #[must_use]
    pub fn new(
        service: EvaluationService<R, S, E>,
        resolver: EvaluationPolicyContextResolver<T, P>,
    ) -> Self {
        Self {
            service,
            resolver,
            tool_router: Self::tool_router(),
        }
    }
    fn validate_json(&self, input: DefinitionInput) -> Result<String> {
        validate_mcp_request(&input).map_err(public_error)?;
        self.resolver
            .resolve_and_authorize(CapabilityV1::EvaluationValidate)
            .map_err(public_error)?;
        let response = match validate_definition(&input.into_core()) {
            Ok(()) => json!({"valid":true,"findings":[]}),
            Err(error) => json!({"valid":false,"error":public_code(error)}),
        };
        serialize(response)
    }
    async fn evaluate_json(&self, input: EvaluateRunInput) -> Result<String> {
        validate_mcp_request(&input).map_err(public_error)?;
        let tenant_id = self
            .resolver
            .resolve_and_authorize(CapabilityV1::EvaluationEvaluate)
            .map_err(public_error)?;
        let definition = input.definition.into_core();
        validate_definition(&definition).map_err(public_error)?;
        validate_evaluate_input(&input.run_id, &definition).map_err(public_error)?;
        let result = match self
            .service
            .evaluate_and_store(&tenant_id, &input.run_id, &definition)
            .await
            .map_err(public_error)?
        {
            CreateOrMatch::Created(result) | CreateOrMatch::Existing(result) => result,
            CreateOrMatch::Conflict => return Err(anyhow::anyhow!("conflict")),
        };
        result_json(&result)
    }
    fn get_json(&self, input: GetResultInput) -> Result<String> {
        validate_mcp_request(&input).map_err(public_error)?;
        let tenant_id = self
            .resolver
            .resolve_and_authorize(CapabilityV1::EvaluationGet)
            .map_err(public_error)?;
        validate_get_input(&input).map_err(public_error)?;
        let key = LogicalEvaluationKey {
            tenant_id: tenant_id.clone(),
            evaluator_id: input.evaluator_id,
            evaluator_version: input.evaluator_version,
            criterion_digest: input.criterion_digest,
            workflow_run_id: input.run_id,
            workflow_revision: input.workflow_revision,
        };
        let result = self
            .service
            .get(&tenant_id, &key)
            .map_err(public_error)?
            .ok_or_else(|| anyhow::anyhow!("not_found"))?;
        result_json(&result)
    }
}
#[tool_router(router = tool_router)]
impl<R, S, E, T, P> EvaluationMcp<R, S, E, T, P>
where
    R: WorkflowEvidenceReader + 'static,
    S: EvaluationStore + 'static,
    E: EvaluationExecutor + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    #[tool(
        name = "evaluation_validate",
        description = "Validate a bounded version-one deterministic evaluation definition."
    )]
    async fn evaluation_validate(&self, Parameters(input): Parameters<DefinitionInput>) -> String {
        tool_response(self.validate_json(input))
    }
    #[tool(
        name = "evaluation_evaluate_run",
        description = "Evaluate one tenant-scoped terminal workflow run without changing it."
    )]
    async fn evaluation_evaluate_run(
        &self,
        Parameters(input): Parameters<EvaluateRunInput>,
    ) -> String {
        tool_response(self.evaluate_json(input).await)
    }
    #[tool(
        name = "evaluation_get_result",
        description = "Get one tenant-scoped immutable evaluation result."
    )]
    async fn evaluation_get_result(&self, Parameters(input): Parameters<GetResultInput>) -> String {
        tool_response(self.get_json(input))
    }
}
#[tool_handler(router = self.tool_router)]
impl<R, S, E, T, P> ServerHandler for EvaluationMcp<R, S, E, T, P>
where
    R: WorkflowEvidenceReader + 'static,
    S: EvaluationStore + 'static,
    E: EvaluationExecutor + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
}
fn validate_evaluate_input(
    run_id: &str,
    definition: &EvaluationDefinitionV1,
) -> Result<(), EvaluationError> {
    validate_logical_key(&LogicalEvaluationKey {
        tenant_id: "tenant".to_owned(),
        evaluator_id: definition.evaluator_id.clone(),
        evaluator_version: definition.evaluator_version.clone(),
        criterion_digest: definition_digest(definition)?,
        workflow_run_id: run_id.to_owned(),
        workflow_revision: 0,
    })
}
fn validate_get_input(input: &GetResultInput) -> Result<(), EvaluationError> {
    validate_logical_key(&LogicalEvaluationKey {
        tenant_id: "tenant".to_owned(),
        evaluator_id: input.evaluator_id.clone(),
        evaluator_version: input.evaluator_version.clone(),
        criterion_digest: input.criterion_digest.clone(),
        workflow_run_id: input.run_id.clone(),
        workflow_revision: input.workflow_revision,
    })
}
fn result_json(result: &crate::EvaluationResultV1) -> Result<String> {
    if result_canonical_bytes(result).map_err(public_error)?.len() > MAX_MCP_RESULT_BYTES {
        return Err(anyhow::anyhow!("limit_exceeded"));
    }
    serialize(
        json!({"evaluator_id":result.logical_key.evaluator_id,"evaluator_version":result.logical_key.evaluator_version,"criterion_digest":result.logical_key.criterion_digest,"run_id":result.logical_key.workflow_run_id,"workflow_revision":result.logical_key.workflow_revision,"evidence_digest":result.evidence_digest,"verdict":verdict_name(result.verdict),"findings":result.findings,"content_hash":result.content_hash}),
    )
}
fn serialize(value: serde_json::Value) -> Result<String> {
    let value = serde_json::to_string(&value).context("could not serialize MCP response")?;
    let escaped_tool_text_bytes = serde_json::to_vec(&value)
        .context("could not measure JSON-escaped MCP tool text")?
        .len();
    (escaped_tool_text_bytes <= MAX_MCP_TOOL_TEXT_BYTES
        && value.len() <= MAX_MCP_SERIALIZED_RESULT_BYTES)
        .then_some(value)
        .ok_or_else(|| anyhow::anyhow!("limit_exceeded"))
}
fn validate_mcp_request<T: Serialize>(input: &T) -> Result<(), EvaluationError> {
    let bytes = serde_json::to_vec(input).map_err(|_| EvaluationError::InvalidRequest)?;
    (bytes.len() <= MAX_MCP_REQUEST_DTO_BYTES)
        .then_some(())
        .ok_or(EvaluationError::LimitExceeded)
}
fn public_error(error: EvaluationError) -> anyhow::Error {
    anyhow::anyhow!(public_code(error))
}
fn public_code(error: EvaluationError) -> &'static str {
    match error.public_code() {
        crate::PublicErrorCode::InvalidRequest => "invalid_request",
        crate::PublicErrorCode::InvalidDefinition => "invalid_definition",
        crate::PublicErrorCode::NotFound => "not_found",
        crate::PublicErrorCode::Conflict => "conflict",
        crate::PublicErrorCode::LimitExceeded => "limit_exceeded",
        crate::PublicErrorCode::OperationFailed => "operation_failed",
    }
}
fn verdict_name(verdict: crate::Verdict) -> &'static str {
    match verdict {
        crate::Verdict::Pass => "pass",
        crate::Verdict::Fail => "fail",
        crate::Verdict::Error => "error",
    }
}
fn tool_response(response: Result<String>) -> String {
    response.unwrap_or_else(|error| { let code = error.to_string(); json!({"error":if matches!(code.as_str(), "invalid_request" | "invalid_definition" | "not_found" | "conflict" | "limit_exceeded" | "operation_failed") { code.as_str() } else { "operation_failed" }}).to_string() })
}
#[must_use]
pub const fn tool_names() -> [&'static str; 3] {
    EVALUATION_TOOLS
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use policy::{
        CorrelationId, GrantV1, PrincipalId, RequestId, TenantId, allow_decision, deny_decision,
    };
    use serde_json::json;

    use super::*;
    use crate::{
        EvaluationResultV1, EvaluatorAssessmentV1, EvaluatorDescriptorV1, ExecutorGuaranteesV1,
        StoreGuaranteesV1, TerminalEvidenceSnapshotV1, TerminalReason, TerminalStatus, Verdict,
    };

    #[derive(Clone, Copy)]
    struct TestExecutor;
    impl EvaluationExecutor for TestExecutor {
        fn descriptor(&self) -> EvaluatorDescriptorV1 {
            EvaluatorDescriptorV1 {
                backend: "test_deterministic",
                version: "v1",
            }
        }
        fn guarantees(&self) -> ExecutorGuaranteesV1 {
            ExecutorGuaranteesV1 {
                deterministic: true,
                ordered_findings: true,
                runtime_required: false,
                external_io: false,
                network_access: false,
                model_judging: false,
                framework_backed: false,
            }
        }
        fn assess<'a>(
            &'a self,
            definition: &'a EvaluationDefinitionV1,
            evidence: &'a TerminalEvidenceSnapshotV1,
        ) -> crate::EvaluationFuture<'a> {
            Box::pin(async move {
                Ok(crate::service::deterministic_assessment(
                    definition, evidence,
                ))
            })
        }
    }

    #[derive(Clone, Default)]
    struct Calls(Arc<Mutex<Vec<&'static str>>>);
    impl Calls {
        fn push(&self, value: &'static str) {
            self.0.lock().expect("calls").push(value);
        }
        fn values(&self) -> Vec<&'static str> {
            self.0.lock().expect("calls").clone()
        }
    }
    #[derive(Clone)]
    struct Source {
        result: std::result::Result<TrustedContextV1, EvaluationError>,
        calls: Calls,
    }
    impl TrustedContextSource for Source {
        fn resolve(&self) -> std::result::Result<TrustedContextV1, EvaluationError> {
            self.calls.push("source");
            self.result.clone()
        }
    }
    #[derive(Clone)]
    struct Policy {
        allow: bool,
        tamper: bool,
        calls: Arc<Mutex<Vec<CapabilityV1>>>,
    }
    impl PolicyResolver for Policy {
        fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
            self.calls.lock().expect("policy").push(request.capability);
            if !self.allow {
                return deny_decision();
            }
            let grant =
                GrantV1::new(Vec::<String>::new(), false, false, false, false).expect("grant");
            let mut decision = allow_decision(&request, &grant).expect("allow");
            if self.tamper {
                let AuthorizationDecisionV1::Allow {
                    decision_digest, ..
                } = &mut decision
                else {
                    unreachable!()
                };
                *decision_digest = "tampered".into();
            }
            decision
        }
    }
    #[derive(Clone)]
    struct Domain {
        calls: Calls,
        snapshot: Option<TerminalEvidenceSnapshotV1>,
        stored: Arc<Mutex<Vec<EvaluationResultV1>>>,
    }
    impl WorkflowEvidenceReader for Domain {
        fn get_terminal(
            &self,
            _: &str,
            _: &str,
        ) -> std::result::Result<Option<TerminalEvidenceSnapshotV1>, EvaluationError> {
            self.calls.push("reader");
            Ok(self.snapshot.clone())
        }
    }
    impl EvaluationStore for Domain {
        fn create_or_match(
            &self,
            result: EvaluationResultV1,
        ) -> std::result::Result<CreateOrMatch, EvaluationError> {
            self.calls.push("store.create");
            self.stored.lock().expect("stored").push(result.clone());
            Ok(CreateOrMatch::Created(result))
        }
        fn get(
            &self,
            _: &str,
            key: &LogicalEvaluationKey,
        ) -> std::result::Result<Option<EvaluationResultV1>, EvaluationError> {
            self.calls.push("store.get");
            Ok(self
                .stored
                .lock()
                .expect("stored")
                .iter()
                .find(|result| &result.logical_key == key)
                .cloned())
        }
        fn list(&self, _: &str) -> std::result::Result<Vec<EvaluationResultV1>, EvaluationError> {
            Ok(vec![])
        }
        fn guarantees(&self) -> StoreGuaranteesV1 {
            StoreGuaranteesV1 {
                durable_across_restart: false,
                visible_across_processes: false,
                crash_atomic: false,
                evicts_on_capacity: false,
                max_results_per_tenant: 1,
                max_results_global: 1,
            }
        }
    }
    fn trusted() -> TrustedContextV1 {
        TrustedContextV1 {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            principal_id: PrincipalId::new("principal").expect("principal"),
            request_id: RequestId::new("request").expect("request"),
            correlation_id: CorrelationId::new("correlation").expect("correlation"),
        }
    }
    fn snapshot() -> TerminalEvidenceSnapshotV1 {
        TerminalEvidenceSnapshotV1 {
            tenant_id: "tenant".into(),
            run_id: "run".into(),
            workflow_id: "workflow".into(),
            workflow_version: "1".into(),
            run_revision: 1,
            terminal_status: TerminalStatus::Succeeded,
            terminal_reason: TerminalReason::Completed,
            attempt_id: "attempt".into(),
            agent_id: "agent".into(),
            capability_scope_digest: "a".repeat(64),
            output: "ok".into(),
            events: vec![],
        }
    }
    fn definition() -> DefinitionInput {
        DefinitionInput {
            evaluator_id: "evaluator".into(),
            evaluator_version: "1".into(),
            criteria: vec![CriterionInput::ExactOutput {
                expected: "ok".into(),
            }],
        }
    }
    type Mcp = EvaluationMcp<Domain, Domain, TestExecutor, Source, Policy>;
    fn mcp(
        allow: bool,
        tamper: bool,
        source_result: std::result::Result<TrustedContextV1, EvaluationError>,
    ) -> (Mcp, Domain, Calls, Arc<Mutex<Vec<CapabilityV1>>>) {
        let source_calls = Calls::default();
        let policy_calls = Arc::new(Mutex::new(vec![]));
        let domain = Domain {
            calls: Calls::default(),
            snapshot: Some(snapshot()),
            stored: Arc::new(Mutex::new(vec![])),
        };
        let service = EvaluationService::new(domain.clone(), domain.clone(), TestExecutor);
        let resolver = EvaluationPolicyContextResolver::new(
            Source {
                result: source_result,
                calls: source_calls.clone(),
            },
            Policy {
                allow,
                tamper,
                calls: Arc::clone(&policy_calls),
            },
        );
        (
            EvaluationMcp::new(service, resolver),
            domain,
            source_calls,
            policy_calls,
        )
    }

    #[test]
    fn exact_three_tools_and_closed_schemas_expose_no_identity_policy_or_backend_fields() {
        assert_eq!(
            tool_names(),
            [
                "evaluation_validate",
                "evaluation_evaluate_run",
                "evaluation_get_result"
            ]
        );
        let schemas = [
            serde_json::to_value(schemars::schema_for!(DefinitionInput))
                .expect("definition schema"),
            serde_json::to_value(schemars::schema_for!(EvaluateRunInput)).expect("evaluate schema"),
            serde_json::to_value(schemars::schema_for!(GetResultInput)).expect("get schema"),
        ];
        for schema in schemas {
            assert_eq!(schema["additionalProperties"], false);
            let encoded = schema.to_string();
            for prohibited in [
                "tenant_id",
                "principal_id",
                "request_id",
                "correlation_id",
                "grant",
                "decision_digest",
                "policy",
                "backend",
            ] {
                assert!(!encoded.contains(prohibited), "schema exposed {prohibited}");
            }
        }
        assert!(serde_json::from_value::<DefinitionInput>(json!({"evaluator_id":"evaluator","evaluator_version":"1","criteria":[],"tenant_id":"forged"})).is_err());
        assert_eq!(
            Mcp::evaluation_validate_tool_attr().name.as_ref(),
            "evaluation_validate"
        );
        assert_eq!(
            Mcp::evaluation_evaluate_run_tool_attr().name.as_ref(),
            "evaluation_evaluate_run"
        );
        assert_eq!(
            Mcp::evaluation_get_result_tool_attr().name.as_ref(),
            "evaluation_get_result"
        );
    }

    #[test]
    fn serialized_ceiling_is_exact_and_pre_policy_while_semantics_are_post_policy() {
        assert!(validate_mcp_request(&"x".repeat(MAX_MCP_REQUEST_DTO_BYTES - 2)).is_ok());
        assert_eq!(
            validate_mcp_request(&"x".repeat(MAX_MCP_REQUEST_DTO_BYTES - 1)),
            Err(EvaluationError::LimitExceeded)
        );
        let (server, domain, source_calls, policy_calls) = mcp(true, false, Ok(trusted()));
        let oversized = DefinitionInput {
            evaluator_id: "evaluator".into(),
            evaluator_version: "1".into(),
            criteria: vec![CriterionInput::ExactOutput {
                expected: "x".repeat(MAX_MCP_REQUEST_DTO_BYTES),
            }],
        };
        assert_eq!(
            tool_response(server.validate_json(oversized)),
            r#"{"error":"limit_exceeded"}"#
        );
        assert!(source_calls.values().is_empty());
        assert!(policy_calls.lock().expect("policy").is_empty());
        assert!(domain.calls.values().is_empty());

        let malformed = DefinitionInput {
            evaluator_id: String::new(),
            ..definition()
        };
        let (denied, domain, source_calls, policy_calls) = mcp(false, false, Ok(trusted()));
        assert_eq!(
            tool_response(denied.validate_json(malformed)),
            r#"{"error":"not_found"}"#,
            "denial must not reveal semantic validity"
        );
        assert_eq!(source_calls.values(), ["source"]);
        assert_eq!(
            policy_calls.lock().expect("policy").as_slice(),
            &[CapabilityV1::EvaluationValidate]
        );
        assert!(domain.calls.values().is_empty());

        let (allowed, domain, _, _) = mcp(true, false, Ok(trusted()));
        assert_eq!(
            allowed
                .validate_json(DefinitionInput {
                    evaluator_id: String::new(),
                    ..definition()
                })
                .expect("response"),
            r#"{"error":"invalid_definition","valid":false}"#
        );
        assert!(domain.calls.values().is_empty());
    }

    #[test]
    fn source_deny_and_tamper_fail_before_effects_with_exact_capabilities() {
        for (allow, tamper, source, expected) in [
            (
                true,
                false,
                Err(EvaluationError::AdapterFailure),
                "operation_failed",
            ),
            (false, false, Ok(trusted()), "not_found"),
            (true, true, Ok(trusted()), "operation_failed"),
        ] {
            let (server, domain, _, policy_calls) = mcp(allow, tamper, source.clone());
            for (response, capability) in [
                (
                    tool_response(server.validate_json(definition())),
                    CapabilityV1::EvaluationValidate,
                ),
                (
                    tool_response(futures_free(server.evaluate_json(EvaluateRunInput {
                        run_id: "run".into(),
                        definition: definition(),
                    }))),
                    CapabilityV1::EvaluationEvaluate,
                ),
                (
                    tool_response(server.get_json(GetResultInput {
                        evaluator_id: "evaluator".into(),
                        evaluator_version: "1".into(),
                        criterion_digest: "a".repeat(64),
                        run_id: "run".into(),
                        workflow_revision: 1,
                    })),
                    CapabilityV1::EvaluationGet,
                ),
            ] {
                assert!(response.contains(expected));
                if source.is_ok() {
                    assert!(policy_calls.lock().expect("policy").contains(&capability));
                }
            }
            assert!(domain.calls.values().is_empty());
        }
    }

    fn futures_free<F: std::future::Future>(future: F) -> F::Output {
        use std::future::Future;
        use std::pin::Pin;
        use std::task::{Context, Poll, Waker};
        let mut context = Context::from_waker(Waker::noop());
        let mut future = Box::pin(future);
        match Pin::new(&mut future).poll(&mut context) {
            Poll::Ready(value) => value,
            Poll::Pending => panic!("immediate MCP future required a runtime"),
        }
    }

    #[test]
    fn injected_service_executor_is_used_and_projection_is_safe() {
        #[derive(Clone, Copy)]
        struct Failing;
        impl EvaluationExecutor for Failing {
            fn descriptor(&self) -> EvaluatorDescriptorV1 {
                EvaluatorDescriptorV1 {
                    backend: "selected",
                    version: "1",
                }
            }
            fn guarantees(&self) -> ExecutorGuaranteesV1 {
                TestExecutor.guarantees()
            }
            fn assess<'a>(
                &'a self,
                _: &'a EvaluationDefinitionV1,
                _: &'a TerminalEvidenceSnapshotV1,
            ) -> crate::EvaluationFuture<'a> {
                Box::pin(async {
                    Ok(EvaluatorAssessmentV1 {
                        verdict: Verdict::Fail,
                        findings: vec!["criterion_1_failed".into()],
                    })
                })
            }
        }
        let source_calls = Calls::default();
        let policy_calls = Arc::new(Mutex::new(vec![]));
        let domain = Domain {
            calls: Calls::default(),
            snapshot: Some(snapshot()),
            stored: Arc::new(Mutex::new(vec![])),
        };
        let server = EvaluationMcp::new(
            EvaluationService::new(domain.clone(), domain.clone(), Failing),
            EvaluationPolicyContextResolver::new(
                Source {
                    result: Ok(trusted()),
                    calls: source_calls,
                },
                Policy {
                    allow: true,
                    tamper: false,
                    calls: policy_calls,
                },
            ),
        );
        let response = futures_free(server.evaluate_json(EvaluateRunInput {
            run_id: "run".into(),
            definition: definition(),
        }))
        .expect("response");
        assert!(response.contains(r#""verdict":"fail""#));
        assert!(response.contains("criterion_1_failed"));
        for secret in ["tenant_id", "principal", "decision", "backend", "selected"] {
            assert!(!response.contains(secret), "projection leaked {secret}");
        }
        assert_eq!(domain.calls.values(), ["reader", "store.create"]);
    }

    #[test]
    fn raw_and_json_escaped_output_budgets_refuse_oversized_immutable_results() {
        let mut result = EvaluationResultV1 {
            logical_key: LogicalEvaluationKey {
                tenant_id: "tenant".into(),
                evaluator_id: "evaluator".into(),
                evaluator_version: "1".into(),
                criterion_digest: "a".repeat(64),
                workflow_run_id: "run".into(),
                workflow_revision: 1,
            },
            evidence_digest: "b".repeat(64),
            verdict: Verdict::Fail,
            findings: vec!["x".repeat(crate::MAX_FINDING_BYTES); 7],
            content_hash: String::new(),
        };
        result.content_hash = crate::result_digest(&result).expect("hash");
        assert_eq!(
            result_json(&result).expect_err("raw budget").to_string(),
            "limit_exceeded"
        );

        result.findings = vec!["\0".repeat(crate::MAX_FINDING_BYTES); 6];
        result.content_hash = crate::result_digest(&result).expect("hash");
        assert!(result_canonical_bytes(&result).expect("canonical").len() <= MAX_MCP_RESULT_BYTES);
        assert_eq!(
            result_json(&result)
                .expect_err("escaped budget")
                .to_string(),
            "limit_exceeded"
        );

        let outer_expanding_text = "\"\\".repeat(10_000);
        let inner =
            serde_json::to_string(&json!({"finding": outer_expanding_text})).expect("inner JSON");
        assert!(inner.len() < MAX_MCP_TOOL_TEXT_BYTES);
        assert!(inner.len() < MAX_MCP_SERIALIZED_RESULT_BYTES);
        assert!(
            serde_json::to_vec(&inner).expect("escaped tool text").len() > MAX_MCP_TOOL_TEXT_BYTES
        );
        assert_eq!(
            serialize(json!({"finding": outer_expanding_text}))
                .expect_err("outer escaped budget")
                .to_string(),
            "limit_exceeded"
        );
    }

    #[test]
    fn framework_and_adapter_errors_project_only_operation_failed() {
        for error in [
            anyhow::anyhow!("serdes private /path token=secret"),
            public_error(EvaluationError::AdapterFailure),
        ] {
            let response = tool_response(Err(error));
            assert_eq!(response, r#"{"error":"operation_failed"}"#);
            assert!(!response.contains("serdes"));
            assert!(!response.contains("secret"));
        }
    }
}
