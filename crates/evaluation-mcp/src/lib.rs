#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::unused_self)]

//! Bounded MCP control-plane adapter for evaluation operations.

mod stdio_transport;

use anyhow::{Context, Result};
use evaluation_core::{
    CriterionV1, EvaluationDefinitionV1, EvaluationError, EvaluationStore, LogicalEvaluationKey,
    WorkflowEvidenceReader, definition_digest, evaluate_and_store, validate_definition,
    validate_logical_key,
};
use policy_core::{
    AuthorizationDecisionV1, AuthorizationRequestV1, CapabilityV1, PolicyResolver,
    TrustedContextV1, canonical_grant, decision_digest,
};
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::{router::tool::ToolRouter, wrapper::Parameters},
    tool, tool_handler, tool_router,
};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::json;
use stdio_transport::BoundedStdioTransport;

pub const EVALUATION_TOOLS: [&str; 3] = [
    "evaluation_validate",
    "evaluation_evaluate_run",
    "evaluation_get_result",
];
pub const MAX_MCP_REQUEST_BYTES: usize = 65_536;
pub const MAX_MCP_SERIALIZED_RESULT_BYTES: usize = 65_536;

/// Host-owned boundary that derives trusted request context independently of MCP input.
pub trait TrustedContextSource: Send + Sync {
    fn resolve(&self) -> Result<TrustedContextV1, EvaluationError>;
}

/// Joins host-derived trusted identity with a verified closed policy decision.
pub struct EvaluationPolicyContextResolver<T, P> {
    source: T,
    policy: P,
}
impl<T, P> EvaluationPolicyContextResolver<T, P>
where
    T: TrustedContextSource,
    P: PolicyResolver,
{
    #[must_use]
    pub fn new(source: T, policy: P) -> Self {
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
            decision_digest: supplied_digest,
        } = self.policy.authorize(request.clone())
        else {
            return Err(EvaluationError::NotFound);
        };
        let canonical_grant =
            canonical_grant(&effective_grant).map_err(|_| EvaluationError::AdapterFailure)?;
        let expected_digest = decision_digest(
            &request,
            &AuthorizationDecisionV1::Allow {
                effective_grant: canonical_grant,
                decision_digest: String::new(),
            },
        )
        .map_err(|_| EvaluationError::AdapterFailure)?;
        if supplied_digest != expected_digest {
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

pub struct EvaluationMcp<R, S, T, P>
where
    R: WorkflowEvidenceReader,
    S: EvaluationStore,
    T: TrustedContextSource,
    P: PolicyResolver,
{
    reader: R,
    store: S,
    resolver: EvaluationPolicyContextResolver<T, P>,
    tool_router: ToolRouter<Self>,
}
impl<R, S, T, P> EvaluationMcp<R, S, T, P>
where
    R: WorkflowEvidenceReader + 'static,
    S: EvaluationStore + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
    #[must_use]
    pub fn new(reader: R, store: S, resolver: EvaluationPolicyContextResolver<T, P>) -> Self {
        Self {
            reader,
            store,
            resolver,
            tool_router: Self::tool_router(),
        }
    }

    pub async fn serve_stdio(self) -> Result<()> {
        self.serve(BoundedStdioTransport::new(
            tokio::io::stdin(),
            tokio::io::stdout(),
        ))
        .await?
        .waiting()
        .await?;
        Ok(())
    }

    fn validate_json(&self, input: DefinitionInput) -> Result<String> {
        let result = (|| {
            validate_mcp_request(&input)?;
            let definition = input.into_core();
            validate_definition(&definition)?;
            self.resolver
                .resolve_and_authorize(CapabilityV1::EvaluationValidate)?;
            Ok::<(), EvaluationError>(())
        })();
        match result {
            Ok(()) => serialize(json!({"valid":true,"findings":[]})),
            Err(error) => serialize(json!({"valid":false,"error":public_code(error)})),
        }
    }

    fn evaluate_json(&self, input: EvaluateRunInput) -> Result<String> {
        validate_mcp_request(&input).map_err(public_error)?;
        let definition = input.definition.into_core();
        validate_definition(&definition).map_err(public_error)?;
        validate_evaluate_input(&input.run_id, &definition).map_err(public_error)?;
        let tenant_id = self
            .resolver
            .resolve_and_authorize(CapabilityV1::EvaluationEvaluate)
            .map_err(public_error)?;
        let result = evaluate_and_store(
            &self.reader,
            &self.store,
            &tenant_id,
            &input.run_id,
            &definition,
        )
        .map_err(public_error)?;
        let result = match result {
            evaluation_core::CreateOrMatch::Created(result)
            | evaluation_core::CreateOrMatch::Existing(result) => result,
            evaluation_core::CreateOrMatch::Conflict => return Err(anyhow::anyhow!("conflict")),
        };
        result_json(&result)
    }

    fn get_json(&self, input: GetResultInput) -> Result<String> {
        validate_mcp_request(&input).map_err(public_error)?;
        validate_get_input(&input).map_err(public_error)?;
        let tenant_id = self
            .resolver
            .resolve_and_authorize(CapabilityV1::EvaluationGet)
            .map_err(public_error)?;
        let key = LogicalEvaluationKey {
            tenant_id: tenant_id.clone(),
            evaluator_id: input.evaluator_id,
            evaluator_version: input.evaluator_version,
            criterion_digest: input.criterion_digest,
            workflow_run_id: input.run_id,
            workflow_revision: input.workflow_revision,
        };
        let result = self
            .store
            .get(&tenant_id, &key)
            .map_err(public_error)?
            .ok_or_else(|| anyhow::anyhow!("not_found"))?;
        result_json(&result)
    }
}

#[tool_router(router = tool_router)]
impl<R, S, T, P> EvaluationMcp<R, S, T, P>
where
    R: WorkflowEvidenceReader + 'static,
    S: EvaluationStore + 'static,
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
        tool_response(self.evaluate_json(input))
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
impl<R, S, T, P> ServerHandler for EvaluationMcp<R, S, T, P>
where
    R: WorkflowEvidenceReader + 'static,
    S: EvaluationStore + 'static,
    T: TrustedContextSource + 'static,
    P: PolicyResolver + 'static,
{
}

fn validate_evaluate_input(
    run_id: &str,
    definition: &EvaluationDefinitionV1,
) -> Result<(), EvaluationError> {
    let key = LogicalEvaluationKey {
        tenant_id: "tenant".to_owned(),
        evaluator_id: definition.evaluator_id.clone(),
        evaluator_version: definition.evaluator_version.clone(),
        criterion_digest: definition_digest(definition)?,
        workflow_run_id: run_id.to_owned(),
        workflow_revision: 0,
    };
    validate_logical_key(&key)
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
fn result_json(result: &evaluation_core::EvaluationResultV1) -> Result<String> {
    serialize(
        json!({"evaluator_id":result.logical_key.evaluator_id,"evaluator_version":result.logical_key.evaluator_version,"criterion_digest":result.logical_key.criterion_digest,"run_id":result.logical_key.workflow_run_id,"workflow_revision":result.logical_key.workflow_revision,"evidence_digest":result.evidence_digest,"verdict":verdict_name(result.verdict),"findings":result.findings,"content_hash":result.content_hash}),
    )
}
fn serialize(value: serde_json::Value) -> Result<String> {
    let value = serde_json::to_string(&value).context("could not serialize MCP response")?;
    (value.len() <= MAX_MCP_SERIALIZED_RESULT_BYTES)
        .then_some(value)
        .ok_or_else(|| anyhow::anyhow!("limit_exceeded"))
}
fn validate_mcp_request<T: Serialize>(input: &T) -> Result<(), EvaluationError> {
    let bytes = serde_json::to_vec(input).map_err(|_| EvaluationError::InvalidRequest)?;
    (bytes.len() <= MAX_MCP_REQUEST_BYTES)
        .then_some(())
        .ok_or(EvaluationError::LimitExceeded)
}
fn public_error(error: EvaluationError) -> anyhow::Error {
    anyhow::anyhow!(public_code(error))
}
fn public_code(error: EvaluationError) -> &'static str {
    match error.public_code() {
        evaluation_core::PublicErrorCode::InvalidRequest => "invalid_request",
        evaluation_core::PublicErrorCode::InvalidDefinition => "invalid_definition",
        evaluation_core::PublicErrorCode::NotFound => "not_found",
        evaluation_core::PublicErrorCode::Conflict => "conflict",
        evaluation_core::PublicErrorCode::LimitExceeded => "limit_exceeded",
        evaluation_core::PublicErrorCode::OperationFailed => "operation_failed",
    }
}
fn verdict_name(verdict: evaluation_core::Verdict) -> &'static str {
    match verdict {
        evaluation_core::Verdict::Pass => "pass",
        evaluation_core::Verdict::Fail => "fail",
        evaluation_core::Verdict::Error => "error",
    }
}
fn tool_response(response: Result<String>) -> String {
    response.unwrap_or_else(|error| {
        let code = error.to_string();
        json!({"error":if matches!(code.as_str(), "invalid_request" | "invalid_definition" | "not_found" | "conflict" | "limit_exceeded" | "operation_failed") { code.as_str() } else { "operation_failed" }}).to_string()
    })
}
#[must_use]
pub const fn tool_names() -> [&'static str; 3] {
    EVALUATION_TOOLS
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use super::*;
    use policy_core::{
        CorrelationId, GrantV1, PrincipalId, RequestId, TenantId, allow_decision, deny_decision,
    };

    #[derive(Clone, Default)]
    struct Calls(Arc<Mutex<Vec<&'static str>>>);
    impl Calls {
        fn push(&self, call: &'static str) {
            self.0.lock().expect("calls").push(call);
        }
        fn values(&self) -> Vec<&'static str> {
            self.0.lock().expect("calls").clone()
        }
    }
    #[derive(Clone)]
    struct Source {
        value: std::result::Result<TrustedContextV1, EvaluationError>,
        calls: Calls,
    }
    impl TrustedContextSource for Source {
        fn resolve(&self) -> Result<TrustedContextV1, EvaluationError> {
            self.calls.push("source");
            self.value.clone()
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
            let AuthorizationDecisionV1::Allow {
                effective_grant,
                decision_digest,
            } = allow_decision(
                &request,
                &GrantV1::new(Vec::<String>::new(), false, false, false, false).expect("grant"),
            )
            .expect("allow")
            else {
                unreachable!()
            };
            AuthorizationDecisionV1::Allow {
                effective_grant,
                decision_digest: if self.tamper {
                    "0".repeat(64)
                } else {
                    decision_digest
                },
            }
        }
    }
    #[derive(Clone, Default)]
    struct Domain {
        calls: Calls,
    }
    impl WorkflowEvidenceReader for Domain {
        fn get_terminal(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<evaluation_core::TerminalEvidenceSnapshotV1>, EvaluationError> {
            self.calls.push("reader");
            Ok(None)
        }
    }
    impl EvaluationStore for Domain {
        fn create_or_match(
            &self,
            _: evaluation_core::EvaluationResultV1,
        ) -> Result<evaluation_core::CreateOrMatch, EvaluationError> {
            self.calls.push("store.create");
            Err(EvaluationError::AdapterFailure)
        }
        fn get(
            &self,
            _: &str,
            _: &LogicalEvaluationKey,
        ) -> Result<Option<evaluation_core::EvaluationResultV1>, EvaluationError> {
            self.calls.push("store.get");
            Ok(None)
        }
        fn list(
            &self,
            _: &str,
        ) -> Result<Vec<evaluation_core::EvaluationResultV1>, EvaluationError> {
            Ok(vec![])
        }
    }
    type Service = EvaluationMcp<Domain, Domain, Source, Policy>;
    fn trusted() -> TrustedContextV1 {
        TrustedContextV1 {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            principal_id: PrincipalId::new("principal").expect("principal"),
            request_id: RequestId::new("request").expect("request"),
            correlation_id: CorrelationId::new("correlation").expect("correlation"),
        }
    }
    fn service(
        source: std::result::Result<TrustedContextV1, EvaluationError>,
        allow: bool,
        tamper: bool,
    ) -> (Service, Domain, Calls, Arc<Mutex<Vec<CapabilityV1>>>) {
        let source_calls = Calls::default();
        let domain = Domain::default();
        let policy_calls = Arc::new(Mutex::new(vec![]));
        let resolver = EvaluationPolicyContextResolver::new(
            Source {
                value: source,
                calls: source_calls.clone(),
            },
            Policy {
                allow,
                tamper,
                calls: Arc::clone(&policy_calls),
            },
        );
        (
            EvaluationMcp::new(domain.clone(), domain.clone(), resolver),
            domain,
            source_calls,
            policy_calls,
        )
    }
    fn definition() -> DefinitionInput {
        DefinitionInput {
            evaluator_id: "evaluator".to_owned(),
            evaluator_version: "1".to_owned(),
            criteria: vec![],
        }
    }
    fn get() -> GetResultInput {
        GetResultInput {
            evaluator_id: "evaluator".to_owned(),
            evaluator_version: "1".to_owned(),
            criterion_digest: "a".repeat(64),
            run_id: "run".to_owned(),
            workflow_revision: 1,
        }
    }
    #[derive(Clone, Copy)]
    enum Operation {
        Validate,
        Evaluate,
        Get,
    }
    impl Operation {
        const fn capability(self) -> CapabilityV1 {
            match self {
                Self::Validate => CapabilityV1::EvaluationValidate,
                Self::Evaluate => CapabilityV1::EvaluationEvaluate,
                Self::Get => CapabilityV1::EvaluationGet,
            }
        }
    }
    fn call(service: &Service, operation: Operation) -> String {
        match operation {
            Operation::Validate => service.validate_json(definition()).expect("response"),
            Operation::Evaluate => tool_response(service.evaluate_json(EvaluateRunInput {
                run_id: "run".to_owned(),
                definition: definition(),
            })),
            Operation::Get => tool_response(service.get_json(get())),
        }
    }

    #[test]
    fn schemas_are_closed_and_caller_cannot_supply_trusted_identity() {
        assert_eq!(
            tool_names(),
            [
                "evaluation_validate",
                "evaluation_evaluate_run",
                "evaluation_get_result"
            ]
        );
        assert!(serde_json::from_value::<DefinitionInput>(json!({"evaluator_id":"evaluator","evaluator_version":"1","criteria":[],"tenant_id":"forged"})).is_err());
    }
    #[test]
    fn invalid_and_oversized_inputs_are_pre_source_policy_and_domain() {
        for operation in [Operation::Validate, Operation::Evaluate, Operation::Get] {
            let (service, domain, source_calls, policy_calls) = service(Ok(trusted()), true, false);
            let result = match operation {
                Operation::Validate => service.validate_json(DefinitionInput {
                    evaluator_id: String::new(),
                    ..definition()
                }),
                Operation::Evaluate => service.evaluate_json(EvaluateRunInput {
                    run_id: "Invalid".to_owned(),
                    definition: definition(),
                }),
                Operation::Get => service.get_json(GetResultInput {
                    run_id: "Invalid".to_owned(),
                    ..get()
                }),
            };
            assert!(result.is_err() || result.expect("validate response").contains("invalid"));
            assert!(domain.calls.values().is_empty());
            assert!(source_calls.values().is_empty());
            assert!(policy_calls.lock().expect("policy").is_empty());
        }
        let (service, domain, source_calls, policy_calls) = service(Ok(trusted()), true, false);
        assert!(
            service
                .evaluate_json(EvaluateRunInput {
                    run_id: "r".repeat(MAX_MCP_REQUEST_BYTES),
                    definition: definition()
                })
                .is_err()
        );
        assert!(domain.calls.values().is_empty());
        assert!(source_calls.values().is_empty());
        assert!(policy_calls.lock().expect("policy").is_empty());
    }
    #[test]
    fn source_failure_deny_and_tampered_allow_are_pre_domain_for_every_capability() {
        for (source, allow, tamper, expected) in [
            (
                Err(EvaluationError::AdapterFailure),
                true,
                false,
                "operation_failed",
            ),
            (Ok(trusted()), false, false, "not_found"),
            (Ok(trusted()), true, true, "operation_failed"),
        ] {
            for operation in [Operation::Validate, Operation::Evaluate, Operation::Get] {
                let (service, domain, _, policy_calls) = service(source.clone(), allow, tamper);
                assert!(call(&service, operation).contains(expected));
                assert!(domain.calls.values().is_empty());
                if source.is_ok() {
                    assert_eq!(
                        policy_calls.lock().expect("policy").as_slice(),
                        &[operation.capability()]
                    );
                } else {
                    assert!(policy_calls.lock().expect("policy").is_empty());
                }
            }
        }
    }
    #[test]
    fn allowed_paths_authorize_exact_capability_before_domain_and_do_not_leak_context() {
        for operation in [Operation::Validate, Operation::Evaluate, Operation::Get] {
            let (service, domain, _, policy_calls) = service(Ok(trusted()), true, false);
            let response = call(&service, operation);
            assert_eq!(
                policy_calls.lock().expect("policy").as_slice(),
                &[operation.capability()]
            );
            if matches!(operation, Operation::Validate) {
                assert_eq!(response, r#"{"findings":[],"valid":true}"#);
                assert!(domain.calls.values().is_empty());
            } else {
                assert_eq!(response, r#"{"error":"not_found"}"#);
                assert!(!domain.calls.values().is_empty());
            }
            assert!(!response.contains("tenant"));
            assert!(!response.contains("principal"));
            assert!(!response.contains("decision"));
        }
    }
    #[test]
    fn result_projection_omits_trusted_context_and_decision() {
        let result = evaluation_core::EvaluationResultV1 {
            logical_key: LogicalEvaluationKey {
                tenant_id: "secret-tenant".to_owned(),
                evaluator_id: "evaluator".to_owned(),
                evaluator_version: "1".to_owned(),
                criterion_digest: "a".repeat(64),
                workflow_run_id: "run".to_owned(),
                workflow_revision: 1,
            },
            evidence_digest: "b".repeat(64),
            verdict: evaluation_core::Verdict::Pass,
            findings: vec![],
            content_hash: "c".repeat(64),
        };
        let response = result_json(&result).expect("response");
        assert!(!response.contains("secret-tenant"));
        assert!(!response.contains("principal"));
        assert!(!response.contains("decision"));
    }
}
