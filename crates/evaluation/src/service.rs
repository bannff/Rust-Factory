use crate::canonical::{error_name, invalid_evidence_digest};
use crate::validation::is_logical_id;
use crate::{
    CreateOrMatch, CriterionV1, EvaluationDefinitionV1, EvaluationError, EvaluationExecutor,
    EvaluationResultV1, EvaluationStore, EvaluatorAssessmentV1, EvaluatorDescriptorV1,
    ExecutorGuaranteesV1, LogicalEvaluationKey, StoreGuaranteesV1, TerminalEvidenceSnapshotV1,
    Verdict, WorkflowEvidenceReader, definition_digest, result_digest, snapshot_digest,
    validate_assessment, validate_definition, validate_snapshot,
};

pub(crate) fn deterministic_assessment(
    definition: &EvaluationDefinitionV1,
    snapshot: &TerminalEvidenceSnapshotV1,
) -> EvaluatorAssessmentV1 {
    let findings = definition
        .criteria
        .iter()
        .enumerate()
        .filter_map(|(index, criterion)| {
            let passed = match criterion {
                CriterionV1::ExactOutput { expected } => snapshot.output == *expected,
                CriterionV1::EventKindCount { kind, expected } => {
                    snapshot
                        .events
                        .iter()
                        .filter(|event| event.kind == *kind)
                        .count()
                        == *expected as usize
                }
                CriterionV1::EventDataEquals { sequence, expected } => snapshot
                    .events
                    .iter()
                    .find(|event| event.sequence == *sequence)
                    .is_some_and(|event| event.data == *expected),
            };
            (!passed).then(|| format!("criterion_{}_failed", index + 1))
        })
        .collect::<Vec<_>>();
    EvaluatorAssessmentV1 {
        verdict: if findings.is_empty() {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
        findings,
    }
}
fn request_key(
    tenant_id: &str,
    run_id: &str,
    definition: &EvaluationDefinitionV1,
    revision: u64,
) -> Result<LogicalEvaluationKey, EvaluationError> {
    Ok(LogicalEvaluationKey {
        tenant_id: tenant_id.to_owned(),
        evaluator_id: definition.evaluator_id.clone(),
        evaluator_version: definition.evaluator_version.clone(),
        criterion_digest: definition_digest(definition)?,
        workflow_run_id: run_id.to_owned(),
        workflow_revision: revision,
    })
}
fn invalid_result(
    key: LogicalEvaluationKey,
    tenant_id: &str,
    run_id: &str,
    error: &EvaluationError,
) -> Result<EvaluationResultV1, EvaluationError> {
    let mut result = EvaluationResultV1 {
        evidence_digest: invalid_evidence_digest(tenant_id, run_id, &key.criterion_digest, error),
        logical_key: key,
        verdict: Verdict::Error,
        findings: vec![error_name(error).to_owned()],
        content_hash: String::new(),
    };
    result.content_hash = result_digest(&result)?;
    Ok(result)
}
fn checked_request(
    tenant_id: &str,
    run_id: &str,
    definition: &EvaluationDefinitionV1,
) -> Result<(), EvaluationError> {
    if !is_logical_id(tenant_id) || !is_logical_id(run_id) {
        return Err(EvaluationError::InvalidRequest);
    }
    validate_definition(definition)
}
pub fn evaluate<R: WorkflowEvidenceReader>(
    reader: &R,
    tenant_id: &str,
    run_id: &str,
    definition: &EvaluationDefinitionV1,
) -> Result<EvaluationResultV1, EvaluationError> {
    checked_request(tenant_id, run_id, definition)?;
    let snapshot = reader
        .get_terminal(tenant_id, run_id)?
        .ok_or(EvaluationError::NotFound)?;
    let key = request_key(tenant_id, run_id, definition, snapshot.run_revision)?;
    if let Err(error) = validate_snapshot(&snapshot).and_then(|()| {
        (snapshot.tenant_id == tenant_id && snapshot.run_id == run_id)
            .then_some(())
            .ok_or(EvaluationError::MalformedEvidence)
    }) {
        return invalid_result(key, tenant_id, run_id, &error);
    }
    let assessment = deterministic_assessment(definition, &snapshot);
    build_result(key, &snapshot, assessment)
}
pub fn evaluate_and_store<R: WorkflowEvidenceReader, S: EvaluationStore>(
    reader: &R,
    store: &S,
    tenant_id: &str,
    run_id: &str,
    definition: &EvaluationDefinitionV1,
) -> Result<CreateOrMatch, EvaluationError> {
    store.create_or_match(evaluate(reader, tenant_id, run_id, definition)?)
}
fn build_result(
    key: LogicalEvaluationKey,
    snapshot: &TerminalEvidenceSnapshotV1,
    assessment: EvaluatorAssessmentV1,
) -> Result<EvaluationResultV1, EvaluationError> {
    validate_assessment(&assessment)?;
    let mut result = EvaluationResultV1 {
        logical_key: key,
        evidence_digest: snapshot_digest(snapshot)?,
        verdict: assessment.verdict,
        findings: assessment.findings,
        content_hash: String::new(),
    };
    result.content_hash = result_digest(&result)?;
    Ok(result)
}
pub struct EvaluationService<R, S, E> {
    reader: R,
    store: S,
    executor: E,
}
impl<R: WorkflowEvidenceReader, S: EvaluationStore, E: EvaluationExecutor>
    EvaluationService<R, S, E>
{
    #[must_use]
    pub const fn new(reader: R, store: S, executor: E) -> Self {
        Self {
            reader,
            store,
            executor,
        }
    }
    pub async fn evaluate(
        &self,
        tenant_id: &str,
        run_id: &str,
        definition: &EvaluationDefinitionV1,
    ) -> Result<EvaluationResultV1, EvaluationError> {
        checked_request(tenant_id, run_id, definition)?;
        let snapshot = self
            .reader
            .get_terminal(tenant_id, run_id)?
            .ok_or(EvaluationError::NotFound)?;
        let key = request_key(tenant_id, run_id, definition, snapshot.run_revision)?;
        if let Err(error) = validate_snapshot(&snapshot).and_then(|()| {
            (snapshot.tenant_id == tenant_id && snapshot.run_id == run_id)
                .then_some(())
                .ok_or(EvaluationError::MalformedEvidence)
        }) {
            return invalid_result(key, tenant_id, run_id, &error);
        }
        let assessment = self
            .executor
            .assess(definition, &snapshot)
            .await
            .map_err(|_| EvaluationError::AdapterFailure)?;
        build_result(key, &snapshot, assessment).map_err(|_| EvaluationError::AdapterFailure)
    }
    pub async fn evaluate_and_store(
        &self,
        tenant_id: &str,
        run_id: &str,
        definition: &EvaluationDefinitionV1,
    ) -> Result<CreateOrMatch, EvaluationError> {
        self.store
            .create_or_match(self.evaluate(tenant_id, run_id, definition).await?)
    }
    pub fn get(
        &self,
        tenant_id: &str,
        key: &LogicalEvaluationKey,
    ) -> Result<Option<EvaluationResultV1>, EvaluationError> {
        self.store.get(tenant_id, key)
    }
    #[must_use]
    pub fn executor_descriptor(&self) -> EvaluatorDescriptorV1 {
        self.executor.descriptor()
    }
    #[must_use]
    pub fn executor_guarantees(&self) -> ExecutorGuaranteesV1 {
        self.executor.guarantees()
    }
    #[must_use]
    pub fn store_guarantees(&self) -> StoreGuaranteesV1 {
        self.store.guarantees()
    }
}
