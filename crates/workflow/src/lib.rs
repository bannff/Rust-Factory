#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::large_enum_variant)]
#![allow(clippy::needless_pass_by_value)]
#![allow(clippy::too_many_lines)]

//! Transport-independent lifecycle contracts for one bounded local agent invocation.
//!
//! The agent-facing MCP surface lives in [`mcp`] and deterministic local
//! adapters in [`memory`], each behind its own feature, so this crate's
//! default build carries no transport or framework dependency.

#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "memory")]
pub mod memory;

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use agent::{AgentId, EffectiveCapabilityCeilingV1, validate_effective_capability_ceiling};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};

pub const MAX_JSON_INPUT_BYTES: usize = 65_536;
pub const MAX_RUN_KEY_BYTES: usize = 128;
pub const MAX_EVIDENCE_BYTES: usize = 65_536;
pub const MAX_EVIDENCE_CHUNK_BYTES: usize = 4_096;
pub const MAX_EVENTS: usize = 64;
pub const INVOCATION_TIMEOUT: Duration = Duration::from_secs(30);

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct LogicalId(String);
impl LogicalId {
    pub fn new(value: impl Into<String>) -> Result<Self, WorkflowError> {
        let value = value.into();
        let mut bytes = value.bytes();
        let valid = value.len() <= 128
            && matches!(bytes.next(), Some(byte) if byte.is_ascii_lowercase() || byte.is_ascii_digit())
            && bytes.all(|byte| {
                byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
            });
        valid
            .then_some(Self(value))
            .ok_or(WorkflowError::InvalidRequest)
    }
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum WorkflowVersion {
    V1,
}
impl WorkflowVersion {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        "v1"
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RequestContext {
    pub tenant_id: LogicalId,
    pub principal_id: LogicalId,
    pub request_id: LogicalId,
    pub correlation_id: LogicalId,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowBudget {
    pub max_attempts: u8,
    pub max_input_bytes: usize,
    pub max_evidence_bytes: usize,
}
impl Default for WorkflowBudget {
    fn default() -> Self {
        Self {
            max_attempts: 1,
            max_input_bytes: MAX_JSON_INPUT_BYTES,
            max_evidence_bytes: MAX_EVIDENCE_BYTES,
        }
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentStep {
    pub agent_id: AgentId,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowDefinitionV1 {
    pub id: LogicalId,
    pub version: WorkflowVersion,
    pub step: AgentStep,
    pub budget: WorkflowBudget,
}
pub trait WorkflowDefinitionCatalog: Send + Sync {
    fn resolve(
        &self,
        id: &LogicalId,
        version: WorkflowVersion,
    ) -> Result<Option<WorkflowDefinitionV1>, WorkflowError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RunStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
    Cancelled,
}
impl RunStatus {
    #[must_use]
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TerminalReason {
    Completed,
    InvocationFailed,
    Cancelled,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AttemptStatus {
    Running,
    Succeeded,
    Failed,
    Cancelled,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct WorkflowEvent {
    pub sequence: u64,
    pub kind: String,
    pub data: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Attempt {
    pub id: LogicalId,
    pub agent_id: AgentId,
    pub effective_capability_ceiling: EffectiveCapabilityCeilingV1,
    pub policy_decision_digest: String,
    pub capability_scope_digest: Option<String>,
    pub status: AttemptStatus,
    pub result: Option<String>,
    pub error: Option<String>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Run {
    pub id: LogicalId,
    pub context: RequestContext,
    pub workflow_id: LogicalId,
    pub workflow_version: WorkflowVersion,
    pub run_key: String,
    pub input_digest: String,
    pub max_evidence_bytes: usize,
    pub status: RunStatus,
    pub revision: u64,
    pub terminal_reason: Option<TerminalReason>,
    pub attempt: Option<Attempt>,
    pub events: Vec<WorkflowEvent>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunSummary {
    pub id: LogicalId,
    pub workflow_id: LogicalId,
    pub workflow_version: WorkflowVersion,
    pub status: RunStatus,
    pub revision: u64,
    pub terminal_reason: Option<TerminalReason>,
}
impl From<&Run> for RunSummary {
    fn from(run: &Run) -> Self {
        Self {
            id: run.id.clone(),
            workflow_id: run.workflow_id.clone(),
            workflow_version: run.workflow_version,
            status: run.status,
            revision: run.revision,
            terminal_reason: run.terminal_reason,
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StartKey {
    pub tenant_id: LogicalId,
    pub workflow_id: LogicalId,
    pub workflow_version: WorkflowVersion,
    pub run_key: String,
}
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StartIdentity {
    pub key: StartKey,
    pub input_digest: String,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum CreateRun {
    Created(Run),
    Existing(Run),
    Conflict,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Transition {
    pub status: RunStatus,
    pub terminal_reason: Option<TerminalReason>,
    pub attempt: Option<Attempt>,
    pub events: Vec<WorkflowEvent>,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum TransitionResult {
    Applied(Run),
    Conflict,
    NotFound,
}
pub trait WorkflowStore: Send + Sync {
    fn create_or_return(
        &self,
        identity: StartIdentity,
        run: Run,
    ) -> Result<CreateRun, WorkflowError>;
    fn get(&self, tenant_id: &LogicalId, run_id: &LogicalId) -> Result<Option<Run>, WorkflowError>;
    fn list(&self, tenant_id: &LogicalId) -> Result<Vec<Run>, WorkflowError>;
    fn transition(
        &self,
        tenant_id: &LogicalId,
        run_id: &LogicalId,
        expected_revision: u64,
        expected_status: RunStatus,
        transition: Transition,
    ) -> Result<TransitionResult, WorkflowError>;
}

#[derive(Clone, Debug)]
pub struct CancellationSignal(Arc<AtomicBool>);
impl CancellationSignal {
    #[must_use]
    pub fn new() -> Self {
        Self(Arc::new(AtomicBool::new(false)))
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::Release);
    }
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}
impl Default for CancellationSignal {
    fn default() -> Self {
        Self::new()
    }
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvocationPolicy {
    pub effective_capability_ceiling: EffectiveCapabilityCeilingV1,
    pub policy_decision_digest: String,
}
#[derive(Clone, Debug)]
pub struct AgentInvocationRequest {
    pub context: RequestContext,
    pub agent_id: AgentId,
    pub input: String,
    pub attempt_id: LogicalId,
    pub effective_capability_ceiling: EffectiveCapabilityCeilingV1,
    pub policy_decision_digest: String,
    pub downstream_idempotency_key: String,
    pub cancellation: CancellationSignal,
    pub deadline: Instant,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct InvocationEvidence {
    pub kind: String,
    pub data: String,
}
impl InvocationEvidence {
    pub fn new(kind: impl Into<String>, data: impl Into<String>) -> Result<Self, WorkflowError> {
        let evidence = Self {
            kind: kind.into(),
            data: data.into(),
        };
        (!evidence.kind.is_empty()
            && evidence.kind.len() + evidence.data.len() <= MAX_EVIDENCE_CHUNK_BYTES)
            .then_some(evidence)
            .ok_or(WorkflowError::LimitExceeded)
    }
}
pub trait InvocationEvidenceSink {
    fn emit(&mut self, evidence: InvocationEvidence) -> Result<(), WorkflowError>;
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct AgentInvocationResult {
    pub capability_scope_digest: String,
}
pub trait AgentInvoker: Send + Sync {
    fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError>;
    fn invoke(
        &self,
        request: AgentInvocationRequest,
        evidence: &mut dyn InvocationEvidenceSink,
    ) -> Result<AgentInvocationResult, WorkflowError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicErrorCode {
    InvalidRequest,
    InvalidDefinition,
    NotFound,
    RunKeyConflict,
    Conflict,
    LimitExceeded,
    OperationFailed,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum WorkflowError {
    InvalidRequest,
    InvalidDefinition,
    NotFound,
    RunKeyConflict,
    Conflict,
    LimitExceeded,
    AdapterFailure,
}
impl WorkflowError {
    #[must_use]
    pub const fn public_code(&self) -> PublicErrorCode {
        match self {
            Self::InvalidRequest => PublicErrorCode::InvalidRequest,
            Self::InvalidDefinition => PublicErrorCode::InvalidDefinition,
            Self::NotFound => PublicErrorCode::NotFound,
            Self::RunKeyConflict => PublicErrorCode::RunKeyConflict,
            Self::Conflict => PublicErrorCode::Conflict,
            Self::LimitExceeded => PublicErrorCode::LimitExceeded,
            Self::AdapterFailure => PublicErrorCode::OperationFailed,
        }
    }
}
impl fmt::Display for WorkflowError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "workflow operation failed: {:?}", self.public_code())
    }
}
impl std::error::Error for WorkflowError {}

pub fn validate_definition(definition: &WorkflowDefinitionV1) -> Result<(), WorkflowError> {
    (definition.budget.max_attempts == 1
        && definition.budget.max_input_bytes > 0
        && definition.budget.max_input_bytes <= MAX_JSON_INPUT_BYTES
        && definition.budget.max_evidence_bytes > 0
        && definition.budget.max_evidence_bytes <= MAX_EVIDENCE_BYTES)
        .then_some(())
        .ok_or(WorkflowError::InvalidDefinition)
}
pub fn canonical_json(input: &str, max_bytes: usize) -> Result<String, WorkflowError> {
    if input.len() > max_bytes || input.len() > MAX_JSON_INPUT_BYTES {
        return Err(WorkflowError::LimitExceeded);
    }
    let value: Value = serde_json::from_str(input).map_err(|_| WorkflowError::InvalidRequest)?;
    reject_duplicate_keys(input)?;
    serde_json::to_string(&canonical_value(value)).map_err(|_| WorkflowError::InvalidRequest)
}
#[must_use]
pub fn input_digest(canonical_input: &str) -> String {
    hex_digest(canonical_input.as_bytes())
}

pub struct WorkflowRunner<S, C, I> {
    store: S,
    catalog: C,
    invoker: I,
    cancellations: Mutex<BTreeMap<LogicalId, CancellationSignal>>,
}
impl<S: WorkflowStore, C: WorkflowDefinitionCatalog, I: AgentInvoker> WorkflowRunner<S, C, I> {
    #[must_use]
    pub fn new(store: S, catalog: C, invoker: I) -> Self {
        Self {
            store,
            catalog,
            invoker,
            cancellations: Mutex::new(BTreeMap::new()),
        }
    }
    pub fn validate(&self, definition: &WorkflowDefinitionV1) -> Result<(), WorkflowError> {
        validate_definition(definition)?;
        self.invoker
            .validate_agent(&definition.step.agent_id)?
            .then_some(())
            .ok_or(WorkflowError::NotFound)
    }
    pub fn start(
        &self,
        context: RequestContext,
        workflow_id: LogicalId,
        version: WorkflowVersion,
        run_key: String,
        input: String,
    ) -> Result<RunSummary, WorkflowError> {
        self.start_with_policy(
            context,
            workflow_id,
            version,
            run_key,
            input,
            InvocationPolicy {
                effective_capability_ceiling: EffectiveCapabilityCeilingV1 {
                    allowed_tool_ids: vec![],
                    memory_enabled: false,
                    knowledge_enabled: false,
                    sandbox_execution_allowed: false,
                    communication_allowed: false,
                },
                policy_decision_digest: "0".repeat(64),
            },
        )
    }
    pub fn start_with_policy(
        &self,
        context: RequestContext,
        workflow_id: LogicalId,
        version: WorkflowVersion,
        run_key: String,
        input: String,
        policy: InvocationPolicy,
    ) -> Result<RunSummary, WorkflowError> {
        validate_invocation_policy(
            &policy.effective_capability_ceiling,
            &policy.policy_decision_digest,
        )?;
        if run_key.is_empty() || run_key.len() > MAX_RUN_KEY_BYTES {
            return Err(WorkflowError::InvalidRequest);
        }
        let definition = self
            .catalog
            .resolve(&workflow_id, version)?
            .ok_or(WorkflowError::NotFound)?;
        self.validate(&definition)?;
        let canonical_input = canonical_json(&input, definition.budget.max_input_bytes)?;
        let identity = StartIdentity {
            key: StartKey {
                tenant_id: context.tenant_id.clone(),
                workflow_id: workflow_id.clone(),
                workflow_version: version,
                run_key: run_key.clone(),
            },
            input_digest: input_digest(&canonical_input),
        };
        let run = Run {
            id: derived_id("run", &identity_material(&identity))?,
            context: context.clone(),
            workflow_id,
            workflow_version: version,
            run_key,
            input_digest: identity.input_digest.clone(),
            max_evidence_bytes: definition.budget.max_evidence_bytes,
            status: RunStatus::Pending,
            revision: 0,
            terminal_reason: None,
            attempt: None,
            events: vec![],
        };
        match self.store.create_or_return(identity, run)? {
            CreateRun::Existing(run) => Ok(RunSummary::from(&run)),
            CreateRun::Conflict => Err(WorkflowError::RunKeyConflict),
            CreateRun::Created(run) => self.execute(definition, run, canonical_input, policy),
        }
    }
    fn execute(
        &self,
        definition: WorkflowDefinitionV1,
        run: Run,
        input: String,
        policy: InvocationPolicy,
    ) -> Result<RunSummary, WorkflowError> {
        let attempt_id = derived_id("attempt", &format!("{}:1", run.id.as_str()))?;
        let started = Transition {
            status: RunStatus::Running,
            terminal_reason: None,
            attempt: Some(Attempt {
                id: attempt_id.clone(),
                agent_id: definition.step.agent_id.clone(),
                effective_capability_ceiling: policy.effective_capability_ceiling.clone(),
                policy_decision_digest: policy.policy_decision_digest.clone(),
                capability_scope_digest: None,
                status: AttemptStatus::Running,
                result: None,
                error: None,
            }),
            events: vec![WorkflowEvent {
                sequence: 1,
                kind: "started".to_owned(),
                data: String::new(),
            }],
        };
        let active = match self.store.transition(
            &run.context.tenant_id,
            &run.id,
            run.revision,
            RunStatus::Pending,
            started,
        ) {
            Ok(TransitionResult::Applied(value)) => value,
            Ok(TransitionResult::Conflict) => {
                self.unregister(&run.id)?;
                return Err(WorkflowError::Conflict);
            }
            Ok(TransitionResult::NotFound) => {
                self.unregister(&run.id)?;
                return Err(WorkflowError::NotFound);
            }
            Err(error) => {
                self.unregister(&run.id)?;
                return Err(error);
            }
        };
        let signal = CancellationSignal::new();
        if self
            .cancellations
            .lock()
            .map(|mut cancellations| cancellations.insert(active.id.clone(), signal.clone()))
            .is_err()
        {
            let _ = self.store.transition(
                &active.context.tenant_id,
                &active.id,
                active.revision,
                RunStatus::Running,
                finish_failure(&active, "cancellation_registration_failed"),
            );
            return Err(WorkflowError::AdapterFailure);
        }
        let key = hex_digest(
            format!(
                "{}:{}:{}",
                active.context.tenant_id.as_str(),
                active.id.as_str(),
                attempt_id.as_str()
            )
            .as_bytes(),
        );
        let mut evidence = EvidenceCollector::new(active.max_evidence_bytes, active.events.len());
        let outcome = self.invoker.invoke(
            AgentInvocationRequest {
                context: active.context.clone(),
                agent_id: definition.step.agent_id,
                input,
                attempt_id,
                effective_capability_ceiling: policy.effective_capability_ceiling,
                policy_decision_digest: policy.policy_decision_digest,
                downstream_idempotency_key: key,
                cancellation: signal,
                deadline: Instant::now() + INVOCATION_TIMEOUT,
            },
            &mut evidence,
        );
        self.unregister(&active.id)?;
        let (transition, result_error) = match outcome {
            Ok(result) => match finish_success(&active, result, evidence) {
                Ok(transition) => (transition, None),
                Err(error) => (
                    finish_failure(&active, "invalid_invocation_result"),
                    Some(error),
                ),
            },
            Err(error) => (
                finish_failure(
                    &active,
                    if error == WorkflowError::LimitExceeded {
                        "evidence_limit_exceeded"
                    } else {
                        "invocation_failed"
                    },
                ),
                Some(error),
            ),
        };
        let summary = match self.store.transition(
            &active.context.tenant_id,
            &active.id,
            active.revision,
            RunStatus::Running,
            transition,
        )? {
            TransitionResult::Applied(value) => RunSummary::from(&value),
            TransitionResult::Conflict => return self.get(&active.context.tenant_id, active.id),
            TransitionResult::NotFound => return Err(WorkflowError::NotFound),
        };
        match result_error {
            Some(WorkflowError::LimitExceeded) => Err(WorkflowError::LimitExceeded),
            Some(_) | None => Ok(summary),
        }
    }
    fn unregister(&self, run_id: &LogicalId) -> Result<(), WorkflowError> {
        self.cancellations
            .lock()
            .map_err(|_| WorkflowError::AdapterFailure)?
            .remove(run_id);
        Ok(())
    }
    pub fn get(&self, tenant: &LogicalId, run: LogicalId) -> Result<RunSummary, WorkflowError> {
        self.store
            .get(tenant, &run)?
            .map(|value| RunSummary::from(&value))
            .ok_or(WorkflowError::NotFound)
    }
    pub fn list(&self, tenant: &LogicalId) -> Result<Vec<RunSummary>, WorkflowError> {
        Ok(self
            .store
            .list(tenant)?
            .iter()
            .map(RunSummary::from)
            .collect())
    }
    pub fn cancel(
        &self,
        tenant: &LogicalId,
        run_id: LogicalId,
    ) -> Result<RunSummary, WorkflowError> {
        let run = self
            .store
            .get(tenant, &run_id)?
            .ok_or(WorkflowError::NotFound)?;
        if run.status.is_terminal() {
            return Ok(RunSummary::from(&run));
        }
        let signal = self
            .cancellations
            .lock()
            .map_err(|_| WorkflowError::AdapterFailure)?
            .get(&run.id)
            .cloned()
            .ok_or(WorkflowError::Conflict)?;
        signal.cancel();
        let attempt = run.attempt.clone().map(|mut item| {
            item.status = AttemptStatus::Cancelled;
            item
        });
        let cancelled = Transition {
            status: RunStatus::Cancelled,
            terminal_reason: Some(TerminalReason::Cancelled),
            attempt,
            events: vec![WorkflowEvent {
                sequence: run.events.len() as u64 + 1,
                kind: "cancelled".to_owned(),
                data: String::new(),
            }],
        };
        match self
            .store
            .transition(tenant, &run.id, run.revision, RunStatus::Running, cancelled)?
        {
            TransitionResult::Applied(value) => Ok(RunSummary::from(&value)),
            TransitionResult::Conflict => self.get(tenant, run.id),
            TransitionResult::NotFound => Err(WorkflowError::NotFound),
        }
    }
}

struct EvidenceCollector {
    limit: usize,
    events: Vec<InvocationEvidence>,
    output: Option<String>,
}
impl EvidenceCollector {
    fn new(limit: usize, existing_events: usize) -> Self {
        Self {
            limit,
            events: Vec::with_capacity(MAX_EVENTS.saturating_sub(existing_events)),
            output: None,
        }
    }
}
impl InvocationEvidenceSink for EvidenceCollector {
    fn emit(&mut self, evidence: InvocationEvidence) -> Result<(), WorkflowError> {
        if self.events.len() >= MAX_EVENTS.saturating_sub(1)
            || evidence.kind.len() + evidence.data.len() > MAX_EVIDENCE_CHUNK_BYTES
            || self
                .events
                .iter()
                .map(|item| item.kind.len() + item.data.len())
                .sum::<usize>()
                + evidence.kind.len()
                + evidence.data.len()
                > self.limit
        {
            return Err(WorkflowError::LimitExceeded);
        }
        if evidence.kind == "result" {
            if self.output.is_some() {
                return Err(WorkflowError::InvalidRequest);
            }
            self.output = Some(evidence.data.clone());
        }
        self.events.push(evidence);
        Ok(())
    }
}
fn finish_success(
    run: &Run,
    result: AgentInvocationResult,
    evidence: EvidenceCollector,
) -> Result<Transition, WorkflowError> {
    let output = evidence.output.ok_or(WorkflowError::InvalidRequest)?;
    let mut attempt = run.attempt.clone().ok_or(WorkflowError::Conflict)?;
    attempt.status = AttemptStatus::Succeeded;
    attempt.capability_scope_digest = Some(result.capability_scope_digest);
    attempt.result = Some(output);
    let events = evidence
        .events
        .into_iter()
        .enumerate()
        .map(|(index, item)| WorkflowEvent {
            sequence: run.events.len() as u64 + index as u64 + 1,
            kind: item.kind,
            data: item.data,
        })
        .collect();
    Ok(Transition {
        status: RunStatus::Succeeded,
        terminal_reason: Some(TerminalReason::Completed),
        attempt: Some(attempt),
        events,
    })
}
fn finish_failure(run: &Run, error: &str) -> Transition {
    let mut attempt = run.attempt.clone().expect("running run has attempt");
    attempt.status = AttemptStatus::Failed;
    attempt.error = Some(error.to_owned());
    Transition {
        status: RunStatus::Failed,
        terminal_reason: Some(TerminalReason::InvocationFailed),
        attempt: Some(attempt),
        events: vec![WorkflowEvent {
            sequence: run.events.len() as u64 + 1,
            kind: "invocation_failed".to_owned(),
            data: error.to_owned(),
        }],
    }
}
#[must_use]
pub fn transition_is_valid(run: &Run, expected_status: RunStatus, transition: &Transition) -> bool {
    if run.status != expected_status
        || run.status.is_terminal()
        || transition.events.is_empty()
        || run.events.len() + transition.events.len() > MAX_EVENTS
        || run
            .events
            .iter()
            .chain(&transition.events)
            .map(|event| event.kind.len() + event.data.len())
            .sum::<usize>()
            > run.max_evidence_bytes
    {
        return false;
    }
    let mut sequence = run.events.last().map_or(1, |event| event.sequence + 1);
    if transition.events.iter().any(|event| {
        let valid = event.sequence == sequence && !event.kind.is_empty();
        sequence += 1;
        !valid
    }) {
        return false;
    }
    match (
        expected_status,
        transition.status,
        transition.terminal_reason,
        run.attempt.as_ref(),
        transition.attempt.as_ref(),
    ) {
        (RunStatus::Pending, RunStatus::Running, None, None, Some(next)) => {
            validate_invocation_policy(
                &next.effective_capability_ceiling,
                &next.policy_decision_digest,
            )
            .is_ok()
                && next.status == AttemptStatus::Running
                && next.capability_scope_digest.is_none()
                && next.result.is_none()
                && next.error.is_none()
        }
        (
            RunStatus::Running,
            RunStatus::Succeeded,
            Some(TerminalReason::Completed),
            Some(previous),
            Some(next),
        ) => {
            same_attempt(previous, next)
                && next.status == AttemptStatus::Succeeded
                && next.result.is_some()
                && next.error.is_none()
        }
        (
            RunStatus::Running,
            RunStatus::Failed,
            Some(TerminalReason::InvocationFailed),
            Some(previous),
            Some(next),
        ) => {
            same_attempt(previous, next)
                && next.status == AttemptStatus::Failed
                && next.result.is_none()
                && next.error.is_some()
        }
        (
            RunStatus::Running,
            RunStatus::Cancelled,
            Some(TerminalReason::Cancelled),
            Some(previous),
            Some(next),
        ) => {
            same_attempt(previous, next)
                && next.status == AttemptStatus::Cancelled
                && next.result.is_none()
        }
        _ => false,
    }
}
fn same_attempt(previous: &Attempt, next: &Attempt) -> bool {
    previous.id == next.id
        && previous.agent_id == next.agent_id
        && previous.effective_capability_ceiling == next.effective_capability_ceiling
        && previous.policy_decision_digest == next.policy_decision_digest
        && previous.status == AttemptStatus::Running
}
fn validate_invocation_policy(
    ceiling: &EffectiveCapabilityCeilingV1,
    decision_digest: &str,
) -> Result<(), WorkflowError> {
    validate_effective_capability_ceiling(ceiling).map_err(|_| WorkflowError::InvalidRequest)?;
    (decision_digest.len() == 64 && decision_digest.bytes().all(|byte| byte.is_ascii_hexdigit()))
        .then_some(())
        .ok_or(WorkflowError::InvalidRequest)
}
fn identity_material(identity: &StartIdentity) -> String {
    format!(
        "{}\u{0}{}\u{0}{}\u{0}{}\u{0}{}",
        identity.key.tenant_id.as_str(),
        identity.key.workflow_id.as_str(),
        identity.key.workflow_version.as_str(),
        identity.key.run_key,
        identity.input_digest
    )
}
fn derived_id(prefix: &str, source: &str) -> Result<LogicalId, WorkflowError> {
    LogicalId::new(format!("{prefix}-{}", &hex_digest(source.as_bytes())[..32]))
}
fn hex_digest(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
fn canonical_value(value: Value) -> Value {
    match value {
        Value::Array(values) => Value::Array(values.into_iter().map(canonical_value).collect()),
        Value::Object(values) => {
            let mut sorted = Map::new();
            for (key, value) in values {
                sorted.insert(key, canonical_value(value));
            }
            Value::Object(sorted)
        }
        value => value,
    }
}
fn reject_duplicate_keys(input: &str) -> Result<(), WorkflowError> {
    struct Scanner<'a> {
        input: &'a [u8],
        index: usize,
    }
    impl Scanner<'_> {
        fn ws(&mut self) {
            while self.index < self.input.len() && self.input[self.index].is_ascii_whitespace() {
                self.index += 1;
            }
        }
        fn string(&mut self) -> Result<String, WorkflowError> {
            let start = self.index;
            self.index += 1;
            let mut escaped = false;
            while self.index < self.input.len() {
                let byte = self.input[self.index];
                self.index += 1;
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == b'"' {
                    return serde_json::from_slice(&self.input[start..self.index])
                        .map_err(|_| WorkflowError::InvalidRequest);
                }
            }
            Err(WorkflowError::InvalidRequest)
        }
        fn value(&mut self) -> Result<(), WorkflowError> {
            self.ws();
            match self.input.get(self.index) {
                Some(b'"') => {
                    self.string()?;
                }
                Some(b'{') => {
                    self.index += 1;
                    let mut keys = BTreeSet::new();
                    loop {
                        self.ws();
                        if self.input.get(self.index) == Some(&b'}') {
                            self.index += 1;
                            break;
                        }
                        if self.input.get(self.index) != Some(&b'"') {
                            return Err(WorkflowError::InvalidRequest);
                        }
                        if !keys.insert(self.string()?) {
                            return Err(WorkflowError::InvalidRequest);
                        }
                        self.ws();
                        if self.input.get(self.index) != Some(&b':') {
                            return Err(WorkflowError::InvalidRequest);
                        }
                        self.index += 1;
                        self.value()?;
                        self.ws();
                        match self.input.get(self.index) {
                            Some(b',') => self.index += 1,
                            Some(b'}') => {
                                self.index += 1;
                                break;
                            }
                            _ => return Err(WorkflowError::InvalidRequest),
                        }
                    }
                }
                Some(b'[') => {
                    self.index += 1;
                    self.ws();
                    if self.input.get(self.index) == Some(&b']') {
                        self.index += 1;
                        return Ok(());
                    }
                    loop {
                        self.value()?;
                        self.ws();
                        match self.input.get(self.index) {
                            Some(b',') => self.index += 1,
                            Some(b']') => {
                                self.index += 1;
                                break;
                            }
                            _ => return Err(WorkflowError::InvalidRequest),
                        }
                    }
                }
                _ => {
                    let start = self.index;
                    while self.index < self.input.len()
                        && !matches!(self.input[self.index], b',' | b']' | b'}')
                        && !self.input[self.index].is_ascii_whitespace()
                    {
                        self.index += 1;
                    }
                    if start == self.index {
                        return Err(WorkflowError::InvalidRequest);
                    }
                }
            }
            Ok(())
        }
    }
    let mut scanner = Scanner {
        input: input.as_bytes(),
        index: 0,
    };
    scanner.value()?;
    scanner.ws();
    (scanner.index == scanner.input.len())
        .then_some(())
        .ok_or(WorkflowError::InvalidRequest)
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;
    use std::collections::BTreeMap;

    #[derive(Clone, Debug)]
    enum TestJson {
        Null,
        Bool(bool),
        Number(u16),
        String(String),
        Array(Vec<Self>),
        Object(BTreeMap<String, Self>),
    }

    fn json_value_strategy() -> BoxedStrategy<TestJson> {
        let scalar = prop_oneof![
            Just(TestJson::Null),
            any::<bool>().prop_map(TestJson::Bool),
            (0_u16..=10_000).prop_map(TestJson::Number),
            prop::collection::vec(0_u8..36, 0..=8).prop_map(|characters| {
                TestJson::String(
                    characters
                        .into_iter()
                        .map(|character| match character {
                            0..=25 => char::from(b'a' + character),
                            _ => char::from(b'0' + character - 26),
                        })
                        .collect(),
                )
            }),
        ];
        scalar
            .prop_recursive(3, 128, 4, |inner| {
                prop_oneof![
                    prop::collection::vec(inner.clone(), 0..=4).prop_map(TestJson::Array),
                    prop::collection::vec((0_u8..8, inner), 0..=4).prop_map(|entries| {
                        TestJson::Object(
                            entries
                                .into_iter()
                                .map(|(key, value)| (format!("key{key}"), value))
                                .collect(),
                        )
                    }),
                ]
            })
            .boxed()
    }

    fn render_json(value: &TestJson, reverse_objects: bool) -> String {
        match value {
            TestJson::Null => "null".to_owned(),
            TestJson::Bool(value) => value.to_string(),
            TestJson::Number(value) => value.to_string(),
            TestJson::String(value) => serde_json::to_string(value).expect("JSON string"),
            TestJson::Array(values) => format!(
                "[{}]",
                values
                    .iter()
                    .map(|value| render_json(value, reverse_objects))
                    .collect::<Vec<_>>()
                    .join(",")
            ),
            TestJson::Object(values) => {
                let mut entries = values.iter().collect::<Vec<_>>();
                if reverse_objects {
                    entries.reverse();
                }
                format!(
                    "{{{}}}",
                    entries
                        .into_iter()
                        .map(|(key, value)| format!(
                            "{}:{}",
                            serde_json::to_string(key).expect("JSON key"),
                            render_json(value, reverse_objects)
                        ))
                        .collect::<Vec<_>>()
                        .join(",")
                )
            }
        }
    }

    #[test]
    fn canonical_json_sorts_nested_keys_and_rejects_duplicates() {
        assert_eq!(
            canonical_json(
                r#"{"z":{"b":2,"a":1},"a":[{"d":4,"c":3}]}"#,
                MAX_JSON_INPUT_BYTES
            )
            .expect("canonical"),
            r#"{"a":[{"c":3,"d":4}],"z":{"a":1,"b":2}}"#
        );
        assert!(canonical_json(r#"{"a":1,"a":2}"#, MAX_JSON_INPUT_BYTES).is_err());
    }

    #[test]
    fn canonical_digest_is_stable_across_object_order() {
        let first = canonical_json(r#"{"a":1,"b":2}"#, MAX_JSON_INPUT_BYTES).expect("first");
        let second = canonical_json(r#"{"b":2,"a":1}"#, MAX_JSON_INPUT_BYTES).expect("second");
        assert_eq!(input_digest(&first), input_digest(&second));
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            failure_persistence: None,
            ..ProptestConfig::default()
        })]

        #[test]
        fn canonical_json_and_digest_are_invariant_to_object_order(input in json_value_strategy()) {
            let first_input = render_json(&input, false);
            let second_input = render_json(&input, true);
            prop_assert!(first_input.len() <= 4 * 1024);
            prop_assert!(second_input.len() <= 4 * 1024);

            let first = canonical_json(&first_input, 4 * 1024).expect("first canonical JSON");
            let second = canonical_json(&second_input, 4 * 1024).expect("second canonical JSON");
            prop_assert_eq!(&first, &second);
            prop_assert_eq!(input_digest(&first), input_digest(&second));
            prop_assert_eq!(
                canonical_json(&first, 4 * 1024).expect("canonical JSON is idempotent"),
                first
            );
        }
    }
}
