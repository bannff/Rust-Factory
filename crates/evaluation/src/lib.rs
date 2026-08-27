#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::needless_pass_by_value)]

//! Transport-independent deterministic evaluation of terminal workflow evidence.
//!
//! The agent-facing MCP surface lives in [`mcp`] and a deterministic local
//! store in [`memory`], each behind its own feature, so this crate's default
//! build carries no transport or framework dependency.
//!
//! The `WorkflowStoreEvidenceReader` bridge that adapted `workflow::WorkflowStore`
//! to [`WorkflowEvidenceReader`] deliberately does not live here: it would make
//! this package depend on `workflow`, and Cargo prohibits package cycles
//! regardless of features, permanently blocking `workflow` from ever consuming
//! evaluation evidence. It belongs in the composition binary that needs it.
//! Recover it from `crates/evaluation-memory/src/lib.rs` at commit 6f4d3e9.

#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "memory")]
pub mod memory;

use sha2::{Digest, Sha256};
use std::fmt;

pub const MAX_LOGICAL_ID_BYTES: usize = 128;
pub const MAX_EVALUATOR_VERSION_BYTES: usize = 128;
pub const SHA256_HEX_BYTES: usize = 64;
pub const MAX_CRITERIA: usize = 16;
pub const MAX_EVENTS: usize = 64;
pub const MAX_EXPECTED_BYTES: usize = 16 * 1024;
pub const MAX_EVENT_BYTES: usize = 4 * 1024;
pub const MAX_SNAPSHOT_BYTES: usize = 64 * 1024;
pub const MAX_FINDINGS: usize = 32;
pub const MAX_FINDING_BYTES: usize = 4 * 1024;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EvaluationDefinitionV1 {
    pub evaluator_id: String,
    pub evaluator_version: String,
    pub criteria: Vec<CriterionV1>,
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum CriterionV1 {
    ExactOutput { expected: String },
    EventKindCount { kind: String, expected: u32 },
    EventDataEquals { sequence: u64, expected: String },
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct EvidenceEventV1 {
    pub sequence: u64,
    pub kind: String,
    pub data: String,
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TerminalStatus {
    Succeeded,
    Failed,
    Cancelled,
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum TerminalReason {
    Completed,
    InvocationFailed,
    Cancelled,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalEvidenceSnapshotV1 {
    pub tenant_id: String,
    pub run_id: String,
    pub workflow_id: String,
    pub workflow_version: String,
    pub run_revision: u64,
    pub terminal_status: TerminalStatus,
    pub terminal_reason: TerminalReason,
    pub attempt_id: String,
    pub agent_id: String,
    pub capability_scope_digest: String,
    pub output: String,
    pub events: Vec<EvidenceEventV1>,
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LogicalEvaluationKey {
    pub tenant_id: String,
    pub evaluator_id: String,
    pub evaluator_version: String,
    pub criterion_digest: String,
    pub workflow_run_id: String,
    pub workflow_revision: u64,
}
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum Verdict {
    Pass,
    Fail,
    Error,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationResultV1 {
    pub logical_key: LogicalEvaluationKey,
    pub evidence_digest: String,
    pub verdict: Verdict,
    pub findings: Vec<String>,
    pub content_hash: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreateOrMatch {
    Created(EvaluationResultV1),
    Existing(EvaluationResultV1),
    Conflict,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicErrorCode {
    InvalidRequest,
    InvalidDefinition,
    NotFound,
    Conflict,
    LimitExceeded,
    OperationFailed,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationError {
    InvalidRequest,
    InvalidDefinition,
    NotFound,
    Conflict,
    LimitExceeded,
    MalformedEvidence,
    AdapterFailure,
}
impl EvaluationError {
    #[must_use]
    pub const fn public_code(&self) -> PublicErrorCode {
        match self {
            Self::InvalidRequest => PublicErrorCode::InvalidRequest,
            Self::InvalidDefinition => PublicErrorCode::InvalidDefinition,
            Self::NotFound => PublicErrorCode::NotFound,
            Self::Conflict => PublicErrorCode::Conflict,
            Self::LimitExceeded => PublicErrorCode::LimitExceeded,
            Self::MalformedEvidence | Self::AdapterFailure => PublicErrorCode::OperationFailed,
        }
    }
}
impl fmt::Display for EvaluationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "evaluation operation failed: {:?}", self.public_code())
    }
}
impl std::error::Error for EvaluationError {}

pub trait WorkflowEvidenceReader: Send + Sync {
    fn get_terminal(
        &self,
        tenant_id: &str,
        run_id: &str,
    ) -> Result<Option<TerminalEvidenceSnapshotV1>, EvaluationError>;
}
pub trait EvaluationStore: Send + Sync {
    fn create_or_match(&self, result: EvaluationResultV1)
    -> Result<CreateOrMatch, EvaluationError>;
    fn get(
        &self,
        tenant_id: &str,
        key: &LogicalEvaluationKey,
    ) -> Result<Option<EvaluationResultV1>, EvaluationError>;
    fn list(&self, tenant_id: &str) -> Result<Vec<EvaluationResultV1>, EvaluationError>;
}

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
fn validate_snapshot_fields(snapshot: &TerminalEvidenceSnapshotV1) -> Result<(), EvaluationError> {
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
    Ok(())
}
fn validate_and_bound_snapshot(
    snapshot: &TerminalEvidenceSnapshotV1,
) -> Result<usize, EvaluationError> {
    validate_snapshot_fields(snapshot)?;
    let canonical_len = snapshot_canonical_len(snapshot)?;
    (canonical_len <= MAX_SNAPSHOT_BYTES)
        .then_some(canonical_len)
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
pub fn validate_result(result: &EvaluationResultV1) -> Result<(), EvaluationError> {
    validate_result_fields(result)?;
    (is_sha256(&result.content_hash) && result.content_hash == result_digest(result)?)
        .then_some(())
        .ok_or(EvaluationError::InvalidRequest)
}
fn validate_result_fields(result: &EvaluationResultV1) -> Result<(), EvaluationError> {
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
pub fn evaluate<R: WorkflowEvidenceReader>(
    reader: &R,
    tenant_id: &str,
    run_id: &str,
    definition: &EvaluationDefinitionV1,
) -> Result<EvaluationResultV1, EvaluationError> {
    if !is_logical_id(tenant_id) || !is_logical_id(run_id) {
        return Err(EvaluationError::InvalidRequest);
    }
    validate_definition(definition)?;
    let snapshot = reader
        .get_terminal(tenant_id, run_id)?
        .ok_or(EvaluationError::NotFound)?;
    let criterion_digest = definition_digest(definition)?;
    let key = LogicalEvaluationKey {
        tenant_id: tenant_id.to_owned(),
        evaluator_id: definition.evaluator_id.clone(),
        evaluator_version: definition.evaluator_version.clone(),
        criterion_digest,
        workflow_run_id: run_id.to_owned(),
        workflow_revision: snapshot.run_revision,
    };
    let (verdict, findings, evidence_digest) = match validate_snapshot(&snapshot).and_then(|()| {
        (snapshot.tenant_id == tenant_id && snapshot.run_id == run_id)
            .then_some(())
            .ok_or(EvaluationError::MalformedEvidence)
    }) {
        Ok(()) => {
            let (verdict, findings) = criteria_verdict(&snapshot, definition);
            (verdict, findings, snapshot_digest(&snapshot)?)
        }
        Err(error) => (
            Verdict::Error,
            vec![error_name(&error).to_owned()],
            invalid_evidence_digest(tenant_id, run_id, &key.criterion_digest, &error),
        ),
    };
    let mut result = EvaluationResultV1 {
        logical_key: key,
        evidence_digest,
        verdict,
        findings,
        content_hash: String::new(),
    };
    result.content_hash = result_digest(&result)?;
    Ok(result)
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

fn criteria_verdict(
    snapshot: &TerminalEvidenceSnapshotV1,
    definition: &EvaluationDefinitionV1,
) -> (Verdict, Vec<String>) {
    let findings: Vec<_> = definition
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
        .collect();
    (
        if findings.is_empty() {
            Verdict::Pass
        } else {
            Verdict::Fail
        },
        findings,
    )
}
fn snapshot_canonical_len(snapshot: &TerminalEvidenceSnapshotV1) -> Result<usize, EvaluationError> {
    let mut total = 0;
    for value_len in [
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
        add_field_len(&mut total, value_len)?;
    }
    for event in &snapshot.events {
        for value_len in [
            decimal_len_u64(event.sequence),
            event.kind.len(),
            event.data.len(),
        ] {
            add_field_len(&mut total, value_len)?;
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
fn fields(out: &mut Vec<u8>, fields: &[&str]) {
    for field in fields {
        out.extend_from_slice(field.len().to_string().as_bytes());
        out.push(b':');
        out.extend_from_slice(field.as_bytes());
        out.push(b'\n');
    }
}
fn digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn invalid_evidence_digest(
    tenant_id: &str,
    run_id: &str,
    criterion_digest: &str,
    error: &EvaluationError,
) -> String {
    let mut bytes = Vec::with_capacity(
        tenant_id.len() + run_id.len() + criterion_digest.len() + error_name(error).len() + 64,
    );
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
fn is_logical_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    value.len() <= MAX_LOGICAL_ID_BYTES
        && matches!(bytes.next(), Some(byte) if byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        })
}
fn is_evaluator_version(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= MAX_EVALUATOR_VERSION_BYTES
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase()
                || byte.is_ascii_uppercase()
                || byte.is_ascii_digit()
                || matches!(byte, b'.' | b'_' | b'-')
        })
}
fn is_sha256(value: &str) -> bool {
    value.len() == SHA256_HEX_BYTES
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
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
fn error_name(value: &EvaluationError) -> &'static str {
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

    #[derive(Clone)]
    struct Reader(Option<TerminalEvidenceSnapshotV1>);
    impl WorkflowEvidenceReader for Reader {
        fn get_terminal(
            &self,
            _: &str,
            _: &str,
        ) -> Result<Option<TerminalEvidenceSnapshotV1>, EvaluationError> {
            Ok(self.0.clone())
        }
    }

    fn snapshot() -> TerminalEvidenceSnapshotV1 {
        TerminalEvidenceSnapshotV1 {
            tenant_id: "tenant".to_owned(),
            run_id: "run".to_owned(),
            workflow_id: "workflow".to_owned(),
            workflow_version: "v1".to_owned(),
            run_revision: 7,
            terminal_status: TerminalStatus::Succeeded,
            terminal_reason: TerminalReason::Completed,
            attempt_id: "attempt".to_owned(),
            agent_id: "agent".to_owned(),
            capability_scope_digest: "a".repeat(64),
            output: "hello".to_owned(),
            events: vec![
                EvidenceEventV1 {
                    sequence: 1,
                    kind: "started".to_owned(),
                    data: String::new(),
                },
                EvidenceEventV1 {
                    sequence: 2,
                    kind: "result".to_owned(),
                    data: "hello".to_owned(),
                },
            ],
        }
    }
    fn definition() -> EvaluationDefinitionV1 {
        EvaluationDefinitionV1 {
            evaluator_id: "exact".to_owned(),
            evaluator_version: "1".to_owned(),
            criteria: vec![
                CriterionV1::ExactOutput {
                    expected: "hello".to_owned(),
                },
                CriterionV1::EventKindCount {
                    kind: "result".to_owned(),
                    expected: 1,
                },
                CriterionV1::EventDataEquals {
                    sequence: 2,
                    expected: "hello".to_owned(),
                },
            ],
        }
    }

    #[test]
    fn canonical_v1_golden_vectors_are_stable() {
        let snapshot = snapshot();
        let definition = definition();
        assert_eq!(
            String::from_utf8(snapshot_canonical_bytes(&snapshot).expect("bytes")).expect("utf8"),
            "2:v1\n6:tenant\n3:run\n8:workflow\n2:v1\n1:7\n9:succeeded\n9:completed\n7:attempt\n5:agent\n64:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\n5:hello\n1:2\n1:1\n7:started\n0:\n1:2\n6:result\n5:hello\n"
        );
        assert_eq!(
            snapshot_digest(&snapshot).expect("digest"),
            "400d023425c9ee77e3eb9ac40032e0871dcc3eaf6980b743f29fccdc025150eb"
        );
        assert_eq!(
            String::from_utf8(definition_canonical_bytes(&definition).expect("bytes"))
                .expect("utf8"),
            "2:v1\n5:exact\n1:1\n1:3\n12:exact_output\n5:hello\n16:event_kind_count\n6:result\n1:1\n17:event_data_equals\n1:2\n5:hello\n"
        );
        assert_eq!(
            definition_digest(&definition).expect("digest"),
            "5c94014a3ba627135274d1cf4c9b54e2c06af1a24e396d8d6dc3c5f6ab90d401"
        );
        let result =
            evaluate(&Reader(Some(snapshot)), "tenant", "run", &definition).expect("evaluate");
        assert_eq!(
            String::from_utf8(result_canonical_bytes(&result).expect("bytes")).expect("utf8"),
            "2:v1\n6:tenant\n5:exact\n1:1\n64:5c94014a3ba627135274d1cf4c9b54e2c06af1a24e396d8d6dc3c5f6ab90d401\n3:run\n1:7\n64:400d023425c9ee77e3eb9ac40032e0871dcc3eaf6980b743f29fccdc025150eb\n4:pass\n1:0\n"
        );
        assert_eq!(
            result.content_hash,
            "03414bc05e2c0b4aae494cc0fe12473da48fa0922f637e3836662839a5bebe72"
        );
    }

    #[test]
    fn canonical_hashes_change_for_semantic_mutations() {
        let snapshot = snapshot();
        let mut changed_snapshot = snapshot.clone();
        changed_snapshot.output.push('!');
        assert_ne!(
            snapshot_digest(&snapshot).expect("digest"),
            snapshot_digest(&changed_snapshot).expect("digest")
        );
        let definition = definition();
        let mut changed_definition = definition.clone();
        changed_definition.evaluator_version = "2".to_owned();
        assert_ne!(
            definition_digest(&definition).expect("digest"),
            definition_digest(&changed_definition).expect("digest")
        );
        let mut result =
            evaluate(&Reader(Some(snapshot)), "tenant", "run", &definition).expect("evaluate");
        let original = result_digest(&result).expect("digest");
        result.findings.push("changed".to_owned());
        assert_ne!(original, result_digest(&result).expect("digest"));
    }

    #[test]
    fn exact_and_event_criteria_report_all_failures_in_definition_order() {
        let definition = EvaluationDefinitionV1 {
            evaluator_id: "id".to_owned(),
            evaluator_version: "1".to_owned(),
            criteria: vec![
                CriterionV1::ExactOutput {
                    expected: "wrong".to_owned(),
                },
                CriterionV1::EventKindCount {
                    kind: "result".to_owned(),
                    expected: 2,
                },
                CriterionV1::EventDataEquals {
                    sequence: 2,
                    expected: "wrong".to_owned(),
                },
            ],
        };
        let result = evaluate(&Reader(Some(snapshot())), "tenant", "run", &definition)
            .expect("evaluation result");
        assert_eq!(result.verdict, Verdict::Fail);
        assert_eq!(
            result.findings,
            [
                "criterion_1_failed",
                "criterion_2_failed",
                "criterion_3_failed"
            ]
        );
    }

    #[test]
    fn malformed_or_cross_tenant_evidence_is_an_error_verdict() {
        let mut malformed = snapshot();
        malformed.events[1].sequence = 3;
        let result =
            evaluate(&Reader(Some(malformed)), "tenant", "run", &definition()).expect("result");
        assert_eq!(
            (result.verdict, result.findings),
            (Verdict::Error, vec!["malformed_evidence".to_owned()])
        );
        let mut other_tenant = snapshot();
        other_tenant.tenant_id = "other".to_owned();
        let result =
            evaluate(&Reader(Some(other_tenant)), "tenant", "run", &definition()).expect("result");
        assert_eq!(result.verdict, Verdict::Error);
    }

    #[test]
    fn public_snapshot_encoding_rejects_oversized_malformed_evidence() {
        let mut malformed = snapshot();
        malformed.output = "x".repeat(MAX_SNAPSHOT_BYTES * 4);
        malformed.events[0].data = "y".repeat(MAX_SNAPSHOT_BYTES * 4);

        assert_eq!(
            snapshot_canonical_bytes(&malformed),
            Err(EvaluationError::MalformedEvidence)
        );
        assert_eq!(
            snapshot_digest(&malformed),
            Err(EvaluationError::MalformedEvidence)
        );
    }

    #[test]
    fn oversized_malformed_evidence_uses_a_bounded_error_digest() {
        let mut malformed = snapshot();
        malformed.output = "x".repeat(MAX_SNAPSHOT_BYTES * 4);
        malformed.events[0].data = "y".repeat(MAX_SNAPSHOT_BYTES * 4);
        let result = evaluate(
            &Reader(Some(malformed.clone())),
            "tenant",
            "run",
            &definition(),
        )
        .expect("result");

        malformed.output.push('!');
        malformed.events[0].data.push('!');
        let different_result =
            evaluate(&Reader(Some(malformed)), "tenant", "run", &definition()).expect("result");

        assert_eq!(result.verdict, Verdict::Error);
        assert_eq!(result.findings, ["malformed_evidence"]);
        assert!(is_sha256(&result.evidence_digest));
        assert_eq!(result.evidence_digest, different_result.evidence_digest);
    }

    #[test]
    fn limits_and_invalid_terminal_pairs_fail_closed() {
        let overlong = EvaluationDefinitionV1 {
            evaluator_id: "id".to_owned(),
            evaluator_version: "1".to_owned(),
            criteria: vec![CriterionV1::ExactOutput {
                expected: "x".repeat(MAX_EXPECTED_BYTES + 1),
            }],
        };
        assert_eq!(
            validate_definition(&overlong),
            Err(EvaluationError::LimitExceeded)
        );
        let too_many = EvaluationDefinitionV1 {
            evaluator_id: "id".to_owned(),
            evaluator_version: "1".to_owned(),
            criteria: vec![
                CriterionV1::ExactOutput {
                    expected: String::new()
                };
                MAX_CRITERIA + 1
            ],
        };
        assert_eq!(
            validate_definition(&too_many),
            Err(EvaluationError::InvalidDefinition)
        );
        let mut invalid = snapshot();
        invalid.terminal_reason = TerminalReason::Cancelled;
        assert_eq!(
            validate_snapshot(&invalid),
            Err(EvaluationError::MalformedEvidence)
        );
    }
}
