use crate::{
    CriterionV1, EvaluationDefinitionV1, EvaluationError, EvaluationResultV1, LogicalEvaluationKey,
    MAX_CRITERIA, MAX_EVALUATOR_VERSION_BYTES, MAX_EVENT_BYTES, MAX_EVENTS, MAX_EXPECTED_BYTES,
    MAX_FINDING_BYTES, MAX_FINDINGS, MAX_LOGICAL_ID_BYTES, MAX_SNAPSHOT_BYTES, SHA256_HEX_BYTES,
    TerminalEvidenceSnapshotV1, TerminalReason, TerminalStatus, result_digest,
};

pub fn validate_definition(definition: &EvaluationDefinitionV1) -> Result<(), EvaluationError> {
    if !is_logical_id(&definition.evaluator_id)
        || !is_evaluator_version(&definition.evaluator_version)
        || definition.criteria.len() > MAX_CRITERIA
    {
        return Err(EvaluationError::InvalidDefinition);
    }
    for criterion in &definition.criteria {
        match criterion {
            CriterionV1::ExactOutput { expected }
            | CriterionV1::EventDataEquals { expected, .. }
                if expected.len() > MAX_EXPECTED_BYTES =>
            {
                return Err(EvaluationError::LimitExceeded);
            }
            CriterionV1::EventKindCount { kind, .. }
                if kind.is_empty() || kind.len() > MAX_EVENT_BYTES =>
            {
                return Err(EvaluationError::InvalidDefinition);
            }
            _ => {}
        }
    }
    Ok(())
}
pub fn validate_snapshot(snapshot: &TerminalEvidenceSnapshotV1) -> Result<(), EvaluationError> {
    validate_and_bound_snapshot(snapshot).map(|_| ())
}
pub(crate) fn validate_and_bound_snapshot(
    snapshot: &TerminalEvidenceSnapshotV1,
) -> Result<usize, EvaluationError> {
    if !is_logical_id(&snapshot.tenant_id)
        || !is_logical_id(&snapshot.run_id)
        || !is_logical_id(&snapshot.workflow_id)
        || !is_evaluator_version(&snapshot.workflow_version)
        || !is_logical_id(&snapshot.attempt_id)
        || !is_logical_id(&snapshot.agent_id)
        || snapshot.output.len() > MAX_EXPECTED_BYTES
        || snapshot.events.len() > MAX_EVENTS
        || !is_sha256(&snapshot.capability_scope_digest)
        || !matches!(
            (snapshot.terminal_status, snapshot.terminal_reason),
            (TerminalStatus::Succeeded, TerminalReason::Completed)
                | (TerminalStatus::Failed, TerminalReason::InvocationFailed)
                | (TerminalStatus::Cancelled, TerminalReason::Cancelled)
        )
    {
        return Err(EvaluationError::MalformedEvidence);
    }
    for (index, event) in snapshot.events.iter().enumerate() {
        if event.sequence != (index + 1) as u64
            || event.kind.is_empty()
            || event.kind.len() + event.data.len() > MAX_EVENT_BYTES
        {
            return Err(EvaluationError::MalformedEvidence);
        }
    }
    let len = crate::canonical::snapshot_canonical_len(snapshot)?;
    (len <= MAX_SNAPSHOT_BYTES)
        .then_some(len)
        .ok_or(EvaluationError::LimitExceeded)
}
pub fn validate_logical_key(key: &LogicalEvaluationKey) -> Result<(), EvaluationError> {
    (is_logical_id(&key.tenant_id)
        && is_logical_id(&key.evaluator_id)
        && is_evaluator_version(&key.evaluator_version)
        && is_sha256(&key.criterion_digest)
        && is_logical_id(&key.workflow_run_id))
    .then_some(())
    .ok_or(EvaluationError::InvalidRequest)
}
pub fn validate_assessment(
    assessment: &crate::EvaluatorAssessmentV1,
) -> Result<(), EvaluationError> {
    if assessment.findings.len() > MAX_FINDINGS
        || assessment
            .findings
            .iter()
            .any(|finding| finding.len() > MAX_FINDING_BYTES)
    {
        return Err(EvaluationError::AdapterFailure);
    }
    match assessment.verdict {
        crate::Verdict::Pass if assessment.findings.is_empty() => Ok(()),
        crate::Verdict::Fail | crate::Verdict::Error if !assessment.findings.is_empty() => Ok(()),
        _ => Err(EvaluationError::AdapterFailure),
    }
}
pub fn validate_result(result: &EvaluationResultV1) -> Result<(), EvaluationError> {
    validate_result_fields(result)?;
    (is_sha256(&result.content_hash) && result.content_hash == result_digest(result)?)
        .then_some(())
        .ok_or(EvaluationError::InvalidRequest)
}
pub(crate) fn validate_result_fields(result: &EvaluationResultV1) -> Result<(), EvaluationError> {
    validate_logical_key(&result.logical_key)?;
    if !is_sha256(&result.evidence_digest) {
        return Err(EvaluationError::InvalidRequest);
    }
    if result.findings.len() > MAX_FINDINGS
        || result
            .findings
            .iter()
            .any(|finding| finding.len() > MAX_FINDING_BYTES)
    {
        return Err(EvaluationError::LimitExceeded);
    }
    Ok(())
}
pub(crate) fn is_logical_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= MAX_LOGICAL_ID_BYTES
        && matches!(bytes.next(), Some(byte) if byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}
pub(crate) fn is_evaluator_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_EVALUATOR_VERSION_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
}
pub(crate) fn is_sha256(value: &str) -> bool {
    value.len() == SHA256_HEX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}
