//! Canonical V1 encodings and SHA-256 identities.
use crate::validation::{validate_and_bound_snapshot, validate_result_fields};
use crate::{
    CriterionV1, EvaluationDefinitionV1, EvaluationError, EvaluationResultV1,
    TerminalEvidenceSnapshotV1, TerminalReason, TerminalStatus, Verdict, validate_definition,
};
use sha2::{Digest, Sha256};

pub fn snapshot_canonical_bytes(
    snapshot: &TerminalEvidenceSnapshotV1,
) -> Result<Vec<u8>, EvaluationError> {
    let canonical_len = validate_and_bound_snapshot(snapshot)?;
    let mut out = Vec::with_capacity(canonical_len);
    fields(
        &mut out,
        &[
            "v1",
            &snapshot.tenant_id,
            &snapshot.run_id,
            &snapshot.workflow_id,
            &snapshot.workflow_version,
            &snapshot.run_revision.to_string(),
            status_name(snapshot.terminal_status),
            reason_name(snapshot.terminal_reason),
            &snapshot.attempt_id,
            &snapshot.agent_id,
            &snapshot.capability_scope_digest,
            &snapshot.output,
            &snapshot.events.len().to_string(),
        ],
    );
    for event in &snapshot.events {
        fields(
            &mut out,
            &[&event.sequence.to_string(), &event.kind, &event.data],
        );
    }
    Ok(out)
}
pub fn definition_canonical_bytes(
    definition: &EvaluationDefinitionV1,
) -> Result<Vec<u8>, EvaluationError> {
    validate_definition(definition)?;
    let mut out = Vec::new();
    fields(
        &mut out,
        &[
            "v1",
            &definition.evaluator_id,
            &definition.evaluator_version,
            &definition.criteria.len().to_string(),
        ],
    );
    for criterion in &definition.criteria {
        match criterion {
            CriterionV1::ExactOutput { expected } => fields(&mut out, &["exact_output", expected]),
            CriterionV1::EventKindCount { kind, expected } => {
                fields(&mut out, &["event_kind_count", kind, &expected.to_string()]);
            }
            CriterionV1::EventDataEquals { sequence, expected } => fields(
                &mut out,
                &["event_data_equals", &sequence.to_string(), expected],
            ),
        }
    }
    Ok(out)
}
pub fn result_canonical_bytes(result: &EvaluationResultV1) -> Result<Vec<u8>, EvaluationError> {
    validate_result_fields(result)?;
    let key = &result.logical_key;
    let mut out = Vec::new();
    fields(
        &mut out,
        &[
            "v1",
            &key.tenant_id,
            &key.evaluator_id,
            &key.evaluator_version,
            &key.criterion_digest,
            &key.workflow_run_id,
            &key.workflow_revision.to_string(),
            &result.evidence_digest,
            verdict_name(result.verdict),
            &result.findings.len().to_string(),
        ],
    );
    for finding in &result.findings {
        fields(&mut out, &[finding]);
    }
    Ok(out)
}
pub fn snapshot_digest(snapshot: &TerminalEvidenceSnapshotV1) -> Result<String, EvaluationError> {
    Ok(digest(&snapshot_canonical_bytes(snapshot)?))
}
pub fn definition_digest(definition: &EvaluationDefinitionV1) -> Result<String, EvaluationError> {
    Ok(digest(&definition_canonical_bytes(definition)?))
}
pub fn result_digest(result: &EvaluationResultV1) -> Result<String, EvaluationError> {
    Ok(digest(&result_canonical_bytes(result)?))
}
pub(crate) fn invalid_evidence_digest(
    tenant_id: &str,
    run_id: &str,
    criterion_digest: &str,
    error: &EvaluationError,
) -> String {
    let mut bytes =
        Vec::with_capacity(tenant_id.len() + run_id.len() + criterion_digest.len() + 96);
    fields(
        &mut bytes,
        &[
            "invalid_evidence_v1",
            tenant_id,
            run_id,
            criterion_digest,
            error_name(error),
        ],
    );
    digest(&bytes)
}
pub(crate) fn snapshot_canonical_len(
    snapshot: &TerminalEvidenceSnapshotV1,
) -> Result<usize, EvaluationError> {
    let mut total = 0;
    for len in [
        2,
        snapshot.tenant_id.len(),
        snapshot.run_id.len(),
        snapshot.workflow_id.len(),
        snapshot.workflow_version.len(),
        decimal_len_u64(snapshot.run_revision),
        status_name(snapshot.terminal_status).len(),
        reason_name(snapshot.terminal_reason).len(),
        snapshot.attempt_id.len(),
        snapshot.agent_id.len(),
        snapshot.capability_scope_digest.len(),
        snapshot.output.len(),
        decimal_len(snapshot.events.len()),
    ] {
        add_field_len(&mut total, len)?;
    }
    for event in &snapshot.events {
        for len in [
            decimal_len_u64(event.sequence),
            event.kind.len(),
            event.data.len(),
        ] {
            add_field_len(&mut total, len)?;
        }
    }
    Ok(total)
}
fn add_field_len(total: &mut usize, value_len: usize) -> Result<(), EvaluationError> {
    let field_len = decimal_len(value_len)
        .checked_add(2)
        .and_then(|len| len.checked_add(value_len))
        .ok_or(EvaluationError::LimitExceeded)?;
    *total = total
        .checked_add(field_len)
        .ok_or(EvaluationError::LimitExceeded)?;
    Ok(())
}
fn decimal_len(value: usize) -> usize {
    if value == 0 {
        1
    } else {
        value.ilog10() as usize + 1
    }
}
fn decimal_len_u64(value: u64) -> usize {
    if value == 0 {
        1
    } else {
        value.ilog10() as usize + 1
    }
}
fn fields(out: &mut Vec<u8>, values: &[&str]) {
    for value in values {
        out.extend_from_slice(value.len().to_string().as_bytes());
        out.push(b':');
        out.extend_from_slice(value.as_bytes());
        out.push(b'\n');
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn status_name(value: TerminalStatus) -> &'static str {
    match value {
        TerminalStatus::Succeeded => "succeeded",
        TerminalStatus::Failed => "failed",
        TerminalStatus::Cancelled => "cancelled",
    }
}
fn reason_name(value: TerminalReason) -> &'static str {
    match value {
        TerminalReason::Completed => "completed",
        TerminalReason::InvocationFailed => "invocation_failed",
        TerminalReason::Cancelled => "cancelled",
    }
}
fn verdict_name(value: Verdict) -> &'static str {
    match value {
        Verdict::Pass => "pass",
        Verdict::Fail => "fail",
        Verdict::Error => "error",
    }
}
pub(crate) fn error_name(value: &EvaluationError) -> &'static str {
    match value {
        EvaluationError::InvalidRequest => "invalid_request",
        EvaluationError::InvalidDefinition => "invalid_definition",
        EvaluationError::NotFound => "not_found",
        EvaluationError::Conflict => "conflict",
        EvaluationError::LimitExceeded => "limit_exceeded",
        EvaluationError::MalformedEvidence => "malformed_evidence",
        EvaluationError::AdapterFailure => "operation_failed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        CriterionV1, EvaluationDefinitionV1, EvaluationResultV1, EvidenceEventV1,
        LogicalEvaluationKey, TerminalEvidenceSnapshotV1, TerminalReason, TerminalStatus, Verdict,
    };

    #[test]
    fn canonical_v1_golden_vectors_are_stable() {
        let snapshot = TerminalEvidenceSnapshotV1 {
            tenant_id: "tenant".into(),
            run_id: "run".into(),
            workflow_id: "workflow".into(),
            workflow_version: "v1".into(),
            run_revision: 7,
            terminal_status: TerminalStatus::Succeeded,
            terminal_reason: TerminalReason::Completed,
            attempt_id: "attempt".into(),
            agent_id: "agent".into(),
            capability_scope_digest: "a".repeat(64),
            output: "hello".into(),
            events: vec![
                EvidenceEventV1 {
                    sequence: 1,
                    kind: "started".into(),
                    data: String::new(),
                },
                EvidenceEventV1 {
                    sequence: 2,
                    kind: "result".into(),
                    data: "hello".into(),
                },
            ],
        };
        let definition = EvaluationDefinitionV1 {
            evaluator_id: "exact".into(),
            evaluator_version: "1".into(),
            criteria: vec![
                CriterionV1::ExactOutput {
                    expected: "hello".into(),
                },
                CriterionV1::EventKindCount {
                    kind: "result".into(),
                    expected: 1,
                },
                CriterionV1::EventDataEquals {
                    sequence: 2,
                    expected: "hello".into(),
                },
            ],
        };
        assert_eq!(
            String::from_utf8(snapshot_canonical_bytes(&snapshot).expect("bytes")).expect("utf8"),
            "2:v1\n6:tenant\n3:run\n8:workflow\n2:v1\n1:7\n9:succeeded\n9:completed\n7:attempt\n5:agent\n64:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n5:hello\n1:2\n1:1\n7:started\n0:\n1:2\n6:result\n5:hello\n"
        );
        assert_eq!(
            snapshot_digest(&snapshot).expect("digest"),
            "400d023425c9ee77e3eb9ac40032e0871dcc3eaf6980b743f29fccdc025150eb"
        );
        assert_eq!(
            definition_digest(&definition).expect("digest"),
            "5c94014a3ba627135274d1cf4c9b54e2c06af1a24e396d8d6dc3c5f6ab90d401"
        );
        let key = LogicalEvaluationKey {
            tenant_id: "tenant".into(),
            evaluator_id: "exact".into(),
            evaluator_version: "1".into(),
            criterion_digest: definition_digest(&definition).expect("digest"),
            workflow_run_id: "run".into(),
            workflow_revision: 7,
        };
        let result = EvaluationResultV1 {
            logical_key: key,
            evidence_digest: snapshot_digest(&snapshot).expect("digest"),
            verdict: Verdict::Pass,
            findings: vec![],
            content_hash: String::new(),
        };
        assert_eq!(
            result_digest(&result).expect("digest"),
            "03414bc05e2c0b4aae494cc0fe12473da48fa0922f637e3836662839a5bebe72"
        );
    }
}
