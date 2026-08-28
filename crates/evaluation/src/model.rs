//! Evaluation domain models and bounds.

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
pub const MAX_RESULTS_PER_TENANT: usize = 1024;
pub const MAX_RESULTS_GLOBAL: usize = 4096;

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
pub struct EvaluatorAssessmentV1 {
    pub verdict: Verdict,
    pub findings: Vec<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluatorDescriptorV1 {
    pub backend: &'static str,
    pub version: &'static str,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct ExecutorGuaranteesV1 {
    pub deterministic: bool,
    pub ordered_findings: bool,
    pub runtime_required: bool,
    pub external_io: bool,
    pub network_access: bool,
    pub model_judging: bool,
    pub framework_backed: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(clippy::struct_excessive_bools)]
pub struct StoreGuaranteesV1 {
    pub durable_across_restart: bool,
    pub visible_across_processes: bool,
    pub crash_atomic: bool,
    pub evicts_on_capacity: bool,
    pub max_results_per_tenant: usize,
    pub max_results_global: usize,
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
