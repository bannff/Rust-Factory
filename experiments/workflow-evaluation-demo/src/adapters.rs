use std::sync::{Arc, Mutex};
use std::task::{Poll, Waker};
use std::time::Instant;

use agent::{
    CorrelationId, DefinitionError, InvocationContextV1, LocalAgentRuntime, PrincipalId, RequestId,
    TenantId,
};
use evaluation::{
    EvaluationError, EvidenceEventV1, TerminalEvidenceSnapshotV1,
    TerminalReason as EvaluationReason, TerminalStatus as EvaluationStatus, WorkflowEvidenceReader,
};
use workflow::memory::InMemoryWorkflowStore;
use workflow::{
    AgentInvocationRequest, AgentInvocationResult, AgentInvoker, AttemptStatus, InvocationEvidence,
    InvocationEvidenceSink, LogicalId, RunStatus, TerminalReason as WorkflowReason, WorkflowError,
    WorkflowStore,
};

pub struct DenyScheduledSandbox;

impl agent::Sandbox for DenyScheduledSandbox {
    fn execute(&self, _: agent::SandboxRequest) -> Result<String, DefinitionError> {
        Err(DefinitionError::SandboxDenied)
    }
}

pub struct RuntimeInvoker<M, T, MM, K, SB> {
    pub registry:
        agent::AgentRegistry<agent::InMemoryDefinitionStore, agent::StaticReferenceCatalog>,
    pub provider: M,
    pub tools: T,
    pub memory: MM,
    pub knowledge: K,
    pub sandbox: SB,
}

impl<M, T, MM, K, SB> AgentInvoker for RuntimeInvoker<M, T, MM, K, SB>
where
    M: llm_gateway::LlmProvider,
    T: agent::ToolRegistry,
    MM: agent::MemoryStore,
    K: knowledge::KnowledgeIndex,
    SB: agent::Sandbox,
{
    fn validate_agent(&self, id: &agent::AgentId) -> Result<bool, WorkflowError> {
        match self.registry.get(id) {
            Ok(_) => Ok(true),
            Err(DefinitionError::NotFound) => Ok(false),
            Err(error) => Err(map_definition_error(error)),
        }
    }

    fn invoke<'a>(
        &'a self,
        request: AgentInvocationRequest,
        control: llm_gateway::InvocationControl<'a>,
        evidence: &'a mut dyn InvocationEvidenceSink,
    ) -> workflow::AgentInvocationFuture<'a> {
        Box::pin(async move {
            let context = InvocationContextV1::new(
                TenantId::new(request.context.tenant_id.as_str()).map_err(map_definition_error)?,
                PrincipalId::new(request.context.principal_id.as_str())
                    .map_err(map_definition_error)?,
                RequestId::new(request.context.request_id.as_str())
                    .map_err(map_definition_error)?,
                CorrelationId::new(request.context.correlation_id.as_str())
                    .map_err(map_definition_error)?,
            );
            let runtime = LocalAgentRuntime::new(
                &self.registry,
                &self.provider,
                &self.tools,
                &self.memory,
                &self.knowledge,
                &self.sandbox,
            );
            let result = runtime
                .invoke_with_ceiling(
                    context,
                    &request.agent_id,
                    request.input,
                    &request.effective_capability_ceiling,
                    control,
                )
                .await
                .map_err(map_definition_error)?;
            evidence.emit(InvocationEvidence::new("result", result.output)?)?;
            Ok(AgentInvocationResult {
                capability_scope_digest: result.capability_scope_digest,
            })
        })
    }
}

fn map_definition_error(error: DefinitionError) -> WorkflowError {
    match error {
        DefinitionError::InvalidRequest => WorkflowError::InvalidRequest,
        DefinitionError::NotFound => WorkflowError::NotFound,
        DefinitionError::LimitExceeded => WorkflowError::LimitExceeded,
        DefinitionError::Cancelled => WorkflowError::Cancelled,
        DefinitionError::DeadlineExceeded => WorkflowError::DeadlineExceeded,
        DefinitionError::InvalidId
        | DefinitionError::InvalidDefinition
        | DefinitionError::InvalidReference => WorkflowError::InvalidDefinition,
        DefinitionError::ReferenceUnavailable
        | DefinitionError::BuiltinProtected
        | DefinitionError::UnknownTool(_)
        | DefinitionError::ToolDisallowed(_)
        | DefinitionError::MemoryDenied
        | DefinitionError::KnowledgeDenied
        | DefinitionError::SandboxDenied
        | DefinitionError::AdapterFailure => WorkflowError::AdapterFailure,
    }
}

#[derive(Clone)]
pub struct StrictEvidenceReader<S = InMemoryWorkflowStore>(pub S);

impl<S: WorkflowStore> WorkflowEvidenceReader for StrictEvidenceReader<S> {
    fn get_terminal(
        &self,
        tenant_id: &str,
        run_id: &str,
    ) -> Result<Option<TerminalEvidenceSnapshotV1>, EvaluationError> {
        let tenant = LogicalId::new(tenant_id).map_err(|_| EvaluationError::InvalidRequest)?;
        let run_id = LogicalId::new(run_id).map_err(|_| EvaluationError::InvalidRequest)?;
        let Some(run) = self.0.get(&tenant, &run_id).map_err(map_store_error)? else {
            return Ok(None);
        };
        if !run.status.is_terminal() {
            return Ok(None);
        }
        let attempt = run.attempt.ok_or(EvaluationError::MalformedEvidence)?;
        let (terminal_status, terminal_reason) =
            match (run.status, run.terminal_reason, attempt.status) {
                (
                    RunStatus::Succeeded,
                    Some(WorkflowReason::Completed),
                    AttemptStatus::Succeeded,
                ) => (EvaluationStatus::Succeeded, EvaluationReason::Completed),
                (
                    RunStatus::Failed,
                    Some(WorkflowReason::InvocationFailed),
                    AttemptStatus::Failed,
                ) => (EvaluationStatus::Failed, EvaluationReason::InvocationFailed),
                (
                    RunStatus::Cancelled,
                    Some(WorkflowReason::Cancelled),
                    AttemptStatus::Cancelled,
                ) => (EvaluationStatus::Cancelled, EvaluationReason::Cancelled),
                _ => return Err(EvaluationError::MalformedEvidence),
            };
        let capability_scope_digest = attempt
            .capability_scope_digest
            .ok_or(EvaluationError::MalformedEvidence)?;
        let output = attempt.result.ok_or(EvaluationError::MalformedEvidence)?;
        Ok(Some(TerminalEvidenceSnapshotV1 {
            tenant_id: run.context.tenant_id.as_str().to_owned(),
            run_id: run.id.as_str().to_owned(),
            workflow_id: run.workflow_id.as_str().to_owned(),
            workflow_version: run.workflow_version.as_str().to_owned(),
            run_revision: run.revision,
            terminal_status,
            terminal_reason,
            attempt_id: attempt.id.as_str().to_owned(),
            agent_id: attempt.agent_id.as_str().to_owned(),
            capability_scope_digest,
            output,
            events: run
                .events
                .into_iter()
                .map(|event| EvidenceEventV1 {
                    sequence: event.sequence,
                    kind: event.kind,
                    data: event.data,
                })
                .collect(),
        }))
    }
}

fn map_store_error(_: WorkflowError) -> EvaluationError {
    EvaluationError::AdapterFailure
}

pub struct ProjectDeadlineFactory;

impl llm_gateway::DeadlineFactory for ProjectDeadlineFactory {
    fn create(&self, instant: Instant) -> Box<dyn llm_gateway::DeadlineSignal> {
        Box::new(ProjectDeadline::new(instant))
    }
}

struct DeadlineState {
    waker: Option<Waker>,
    worker_started: bool,
}

struct ProjectDeadline {
    instant: Instant,
    state: Arc<Mutex<DeadlineState>>,
}

impl ProjectDeadline {
    fn new(instant: Instant) -> Self {
        Self {
            instant,
            state: Arc::new(Mutex::new(DeadlineState {
                waker: None,
                worker_started: false,
            })),
        }
    }
}

impl llm_gateway::DeadlineSignal for ProjectDeadline {
    fn instant(&self) -> Instant {
        self.instant
    }

    fn is_elapsed(&self) -> bool {
        Instant::now() >= self.instant
    }

    fn elapsed(&self) -> llm_gateway::DeadlineFuture<'_> {
        Box::pin(DeadlineWait {
            instant: self.instant,
            state: Arc::clone(&self.state),
        })
    }
}

struct DeadlineWait {
    instant: Instant,
    state: Arc<Mutex<DeadlineState>>,
}

impl Future for DeadlineWait {
    type Output = ();

    fn poll(self: std::pin::Pin<&mut Self>, context: &mut std::task::Context<'_>) -> Poll<()> {
        if Instant::now() >= self.instant {
            return Poll::Ready(());
        }
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(_) => return Poll::Ready(()),
        };
        state.waker = Some(context.waker().clone());
        if !state.worker_started {
            state.worker_started = true;
            let shared = Arc::clone(&self.state);
            let instant = self.instant;
            std::thread::spawn(move || {
                std::thread::sleep(instant.saturating_duration_since(Instant::now()));
                let waker = shared.lock().ok().and_then(|mut state| state.waker.take());
                if let Some(waker) = waker {
                    waker.wake();
                }
            });
        }
        Poll::Pending
    }
}

pub struct FixedClock;

impl observability::Clock for FixedClock {
    fn now(&self) -> Result<observability::Timestamp, observability::ObservabilityError> {
        Ok(observability::Timestamp::from_unix_nanos(1))
    }
}
