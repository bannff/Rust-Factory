use std::future::{Future, ready};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use agent::{MemoryStore, ToolRegistry};
use evaluation::{
    CreateOrMatch, CriterionV1, EvaluationDefinitionV1, EvaluationError, EvaluationService,
    EvaluationStore, Verdict, WorkflowEvidenceReader,
};
use observability::{
    EventName, Severity, TelemetryContext, TelemetryQueryV1, TelemetryReader, TelemetryService,
    TenantId,
};
use workflow::memory::{InMemoryWorkflowStore, StaticWorkflowCatalog};
use workflow::{
    AgentInvoker, AgentStep, Attempt, AttemptStatus, CreateRun, InvocationEvidence,
    InvocationEvidenceSink, LogicalId, RequestContext, Run, RunStatus, StartIdentity,
    TerminalReason, Transition, TransitionResult, WorkflowBudget, WorkflowDefinitionV1,
    WorkflowError, WorkflowRunner, WorkflowStore, WorkflowVersion,
};

use super::*;

fn poll_immediate<F: Future>(future: F) -> F::Output {
    let mut future = Box::pin(future);
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("deterministic future unexpectedly pending"),
    }
}

fn logical_id(value: &str) -> LogicalId {
    LogicalId::new(value).expect("valid logical id")
}

fn request_context(tenant: &str) -> RequestContext {
    RequestContext {
        tenant_id: logical_id(tenant),
        principal_id: logical_id(PRINCIPAL),
        request_id: logical_id(REQUEST),
        correlation_id: logical_id(CORRELATION),
    }
}

fn evaluation_definition() -> EvaluationDefinitionV1 {
    EvaluationDefinitionV1 {
        evaluator_id: "demo-evaluator".to_owned(),
        evaluator_version: "v1".to_owned(),
        criteria: vec![
            CriterionV1::ExactOutput {
                expected: "done".to_owned(),
            },
            CriterionV1::EventKindCount {
                kind: "result".to_owned(),
                expected: 1,
            },
        ],
    }
}

fn static_invoker() -> RuntimeInvoker<
    llm_gateway::r#static::StaticProvider,
    agent::FixedToolRegistry,
    agent::InMemoryMemoryStore,
    knowledge::r#static::StaticKnowledgeIndex,
    DenyScheduledSandbox,
> {
    let agent_id = agent::AgentId::new(AGENT).expect("agent id");
    let registry = agent::AgentRegistry::new(
        vec![agent_definition(agent_id)],
        agent::InMemoryDefinitionStore::default(),
        agent::StaticReferenceCatalog::new(["static.demo".to_owned()], [], [], []),
    )
    .expect("registry");
    let fixture = llm_gateway::r#static::StaticFixture::new(
        "done",
        vec![],
        None,
        llm_gateway::FinishReason::Stop,
        None,
        llm_gateway::IdempotencyDisposition::Accepted,
    )
    .expect("fixture");
    RuntimeInvoker {
        registry,
        provider: llm_gateway::r#static::StaticProvider::success(fixture),
        tools: agent::FixedToolRegistry::default(),
        memory: agent::InMemoryMemoryStore::default(),
        knowledge: knowledge::r#static::StaticKnowledgeIndex::new(vec![]).expect("knowledge"),
        sandbox: DenyScheduledSandbox,
    }
}

struct NeverDeadline;
impl llm_gateway::DeadlineSignal for NeverDeadline {
    fn instant(&self) -> Instant {
        Instant::now() + Duration::from_secs(60)
    }
    fn is_elapsed(&self) -> bool {
        false
    }
    fn elapsed(&self) -> llm_gateway::DeadlineFuture<'_> {
        Box::pin(std::future::pending())
    }
}

struct NeverDeadlineFactory;
impl llm_gateway::DeadlineFactory for NeverDeadlineFactory {
    fn create(&self, _: Instant) -> Box<dyn llm_gateway::DeadlineSignal> {
        Box::new(NeverDeadline)
    }
}

fn completed_workflow() -> (InMemoryWorkflowStore, workflow::RunSummary) {
    let agent_id = agent::AgentId::new(AGENT).expect("agent id");
    let workflow_id = logical_id(WORKFLOW);
    let definition = WorkflowDefinitionV1 {
        id: workflow_id.clone(),
        version: WorkflowVersion::V1,
        step: AgentStep { agent_id },
        budget: WorkflowBudget {
            max_attempts: 1,
            max_input_bytes: agent::MAX_INPUT_BYTES,
            max_evidence_bytes: workflow::MAX_EVIDENCE_BYTES,
        },
    };
    let store = InMemoryWorkflowStore::default();
    let runner = WorkflowRunner::new(
        store.clone(),
        StaticWorkflowCatalog::new([definition]),
        static_invoker(),
        Box::new(NeverDeadlineFactory),
        Box::new(llm_gateway::tokio_cancellation::TokioCancellationSignalFactory),
    );
    let summary = poll_immediate(runner.start(
        request_context(TENANT),
        workflow_id,
        WorkflowVersion::V1,
        "demo-run".to_owned(),
        "{}".to_owned(),
    ))
    .expect("workflow succeeds");
    (store, summary)
}

#[test]
fn full_static_done_agent_workflow_evaluation_path_succeeds() {
    let (workflow_store, summary) = completed_workflow();
    assert_eq!(summary.status, RunStatus::Succeeded);
    assert_eq!(summary.terminal_reason, Some(TerminalReason::Completed));

    let run = workflow_store
        .get(&logical_id(TENANT), &summary.id)
        .expect("store read")
        .expect("stored run");
    let attempt = run.attempt.expect("terminal attempt");
    assert_eq!(attempt.status, AttemptStatus::Succeeded);
    assert_eq!(attempt.result.as_deref(), Some("done"));
    assert_eq!(
        run.events
            .iter()
            .map(|event| (event.kind.as_str(), event.data.as_str()))
            .collect::<Vec<_>>(),
        [("started", ""), ("result", "done")]
    );

    let service = EvaluationService::new(
        StrictEvidenceReader(workflow_store),
        evaluation::memory::InMemoryEvaluationStore::new(),
        evaluation::local::DeterministicCriteriaEvaluator,
    );
    let result =
        poll_immediate(service.evaluate(TENANT, summary.id.as_str(), &evaluation_definition()))
            .expect("evaluation succeeds");
    assert_eq!(result.verdict, Verdict::Pass);
    assert!(result.findings.is_empty());
}

#[test]
fn evaluation_replay_matches_and_cloned_stores_share_results() {
    let (workflow_store, summary) = completed_workflow();
    let evaluation_store = evaluation::memory::InMemoryEvaluationStore::new();
    let observer = evaluation_store.clone();
    let service = EvaluationService::new(
        StrictEvidenceReader(workflow_store.clone()),
        evaluation_store,
        evaluation::local::DeterministicCriteriaEvaluator,
    );

    let first = poll_immediate(service.evaluate_and_store(
        TENANT,
        summary.id.as_str(),
        &evaluation_definition(),
    ))
    .expect("first evaluation");
    let created = match first {
        CreateOrMatch::Created(result) => result,
        other => panic!("expected creation, got {other:?}"),
    };
    let replay = poll_immediate(service.evaluate_and_store(
        TENANT,
        summary.id.as_str(),
        &evaluation_definition(),
    ))
    .expect("replayed evaluation");
    assert_eq!(replay, CreateOrMatch::Existing(created.clone()));
    assert_eq!(
        observer.list(TENANT).expect("shared clone read"),
        vec![created]
    );
    assert!(
        StrictEvidenceReader(workflow_store)
            .get_terminal(TENANT, summary.id.as_str())
            .expect("shared workflow clone read")
            .is_some()
    );
}

#[test]
fn strict_evidence_reader_hides_other_tenants_and_missing_runs() {
    let (store, summary) = completed_workflow();
    let reader = StrictEvidenceReader(store);
    assert_eq!(
        reader
            .get_terminal("other-tenant", summary.id.as_str())
            .expect("foreign read"),
        None
    );
    assert_eq!(
        reader
            .get_terminal(TENANT, "missing-run")
            .expect("missing read"),
        None
    );
}

#[derive(Clone)]
struct FixedRunStore(Option<Run>);
impl WorkflowStore for FixedRunStore {
    fn create_or_return(&self, _: StartIdentity, _: Run) -> Result<CreateRun, WorkflowError> {
        panic!("unused create")
    }
    fn get(&self, _: &LogicalId, _: &LogicalId) -> Result<Option<Run>, WorkflowError> {
        Ok(self.0.clone())
    }
    fn list(&self, _: &LogicalId) -> Result<Vec<Run>, WorkflowError> {
        panic!("unused list")
    }
    fn transition(
        &self,
        _: &LogicalId,
        _: &LogicalId,
        _: u64,
        _: RunStatus,
        _: Transition,
    ) -> Result<TransitionResult, WorkflowError> {
        panic!("unused transition")
    }
}

fn raw_run(status: RunStatus, attempt: Option<Attempt>) -> Run {
    Run {
        id: logical_id("run"),
        context: request_context(TENANT),
        workflow_id: logical_id(WORKFLOW),
        workflow_version: WorkflowVersion::V1,
        run_key: "key".to_owned(),
        input_digest: "a".repeat(64),
        max_evidence_bytes: workflow::MAX_EVIDENCE_BYTES,
        status,
        revision: 1,
        terminal_reason: (status == RunStatus::Succeeded).then_some(TerminalReason::Completed),
        attempt,
        events: vec![],
    }
}

#[test]
fn strict_reader_returns_none_for_nonterminal_and_rejects_malformed_terminal_without_synthesis() {
    let nonterminal = StrictEvidenceReader(FixedRunStore(Some(raw_run(RunStatus::Running, None))));
    assert_eq!(nonterminal.get_terminal(TENANT, "run").expect("read"), None);

    let malformed = StrictEvidenceReader(FixedRunStore(Some(raw_run(RunStatus::Succeeded, None))));
    assert_eq!(
        malformed.get_terminal(TENANT, "run"),
        Err(EvaluationError::MalformedEvidence)
    );
    let store = evaluation::memory::InMemoryEvaluationStore::new();
    let observer = store.clone();
    let service = EvaluationService::new(
        malformed,
        store,
        evaluation::local::DeterministicCriteriaEvaluator,
    );
    assert_eq!(
        poll_immediate(service.evaluate_and_store(TENANT, "run", &evaluation_definition())),
        Err(EvaluationError::MalformedEvidence)
    );
    assert!(
        observer
            .list(TENANT)
            .expect("no synthetic result")
            .is_empty()
    );
}

#[derive(Default)]
struct EvidenceSink(Vec<InvocationEvidence>);
impl InvocationEvidenceSink for EvidenceSink {
    fn emit(&mut self, item: InvocationEvidence) -> Result<(), WorkflowError> {
        self.0.push(item);
        Ok(())
    }
}

struct Signal(bool);
impl llm_gateway::CancellationSignal for Signal {
    fn is_cancelled(&self) -> bool {
        self.0
    }
    fn cancelled(&self) -> llm_gateway::CancellationFuture<'_> {
        if self.0 {
            Box::pin(ready(()))
        } else {
            Box::pin(std::future::pending())
        }
    }
}

struct ControlledDeadline(bool);
impl llm_gateway::DeadlineSignal for ControlledDeadline {
    fn instant(&self) -> Instant {
        Instant::now()
    }
    fn is_elapsed(&self) -> bool {
        self.0
    }
    fn elapsed(&self) -> llm_gateway::DeadlineFuture<'_> {
        if self.0 {
            Box::pin(ready(()))
        } else {
            Box::pin(std::future::pending())
        }
    }
}

fn invoke_with_control(
    cancelled: bool,
    elapsed: bool,
) -> Result<workflow::AgentInvocationResult, WorkflowError> {
    let invoker = static_invoker();
    let key = llm_gateway::IdempotencyKey::new("controlled-key").expect("key");
    let cancellation = Signal(cancelled);
    let deadline = ControlledDeadline(elapsed);
    let mut evidence = EvidenceSink::default();
    let result = poll_immediate(invoker.invoke(
        workflow::AgentInvocationRequest {
            context: request_context(TENANT),
            agent_id: agent::AgentId::new(AGENT).expect("agent"),
            input: "{}".to_owned(),
            attempt_id: logical_id("attempt"),
            effective_capability_ceiling: agent::EffectiveCapabilityCeilingV1 {
                allowed_tool_ids: vec![],
                memory_enabled: false,
                knowledge_enabled: false,
                sandbox_execution_allowed: false,
                communication_allowed: false,
            },
            policy_decision_digest: "0".repeat(64),
        },
        llm_gateway::InvocationControl {
            idempotency_key: &key,
            cancellation: &cancellation,
            deadline: &deadline,
        },
        &mut evidence,
    ));
    assert!(
        evidence.0.is_empty(),
        "failed preflight must emit no evidence"
    );
    result
}

#[test]
fn cancellation_and_expired_deadline_categories_are_preserved_without_sleeping() {
    assert_eq!(
        invoke_with_control(true, false),
        Err(WorkflowError::Cancelled)
    );
    assert_eq!(
        invoke_with_control(false, true),
        Err(WorkflowError::DeadlineExceeded)
    );
}

#[derive(Clone, Default)]
struct CallCounts {
    tools: Arc<AtomicUsize>,
    memory: Arc<AtomicUsize>,
    knowledge: Arc<AtomicUsize>,
    sandbox: Arc<AtomicUsize>,
}
struct CountingTools(CallCounts);
impl ToolRegistry for CountingTools {
    fn resolve(&self, _: &str) -> Result<agent::ToolDescriptor, agent::DefinitionError> {
        self.0.tools.fetch_add(1, Ordering::Relaxed);
        Err(agent::DefinitionError::AdapterFailure)
    }
    fn invoke(
        &self,
        _: &agent::ToolDescriptor,
        _: agent::ToolRequest,
    ) -> Result<String, agent::DefinitionError> {
        self.0.tools.fetch_add(1, Ordering::Relaxed);
        Err(agent::DefinitionError::AdapterFailure)
    }
}
struct CountingMemory(CallCounts);
impl MemoryStore for CountingMemory {
    fn recall(&self, _: agent::MemoryRequest) -> Result<Vec<String>, agent::DefinitionError> {
        self.0.memory.fetch_add(1, Ordering::Relaxed);
        Err(agent::DefinitionError::AdapterFailure)
    }
    fn write(&self, _: agent::MemoryRequest, _: String) -> Result<(), agent::DefinitionError> {
        self.0.memory.fetch_add(1, Ordering::Relaxed);
        Err(agent::DefinitionError::AdapterFailure)
    }
}
struct CountingKnowledge(CallCounts);
impl knowledge::KnowledgeIndex for CountingKnowledge {
    fn search(
        &self,
        _: &knowledge::SearchRequest,
    ) -> Result<Vec<knowledge::KnowledgeDocument>, knowledge::KnowledgeError> {
        self.0.knowledge.fetch_add(1, Ordering::Relaxed);
        Err(knowledge::KnowledgeError::Unavailable)
    }
}
struct CountingSandbox(CallCounts);
impl agent::Sandbox for CountingSandbox {
    fn execute(&self, _: agent::SandboxRequest) -> Result<String, agent::DefinitionError> {
        self.0.sandbox.fetch_add(1, Ordering::Relaxed);
        Err(agent::DefinitionError::AdapterFailure)
    }
}

#[test]
fn disabled_capability_ceiling_never_touches_tools_memory_knowledge_or_sandbox() {
    let base = static_invoker();
    let counts = CallCounts::default();
    let invoker = RuntimeInvoker {
        registry: base.registry,
        provider: base.provider,
        tools: CountingTools(counts.clone()),
        memory: CountingMemory(counts.clone()),
        knowledge: CountingKnowledge(counts.clone()),
        sandbox: CountingSandbox(counts.clone()),
    };
    let key = llm_gateway::IdempotencyKey::new("key").expect("key");
    let cancellation = Signal(false);
    let deadline = ControlledDeadline(false);
    let mut evidence = EvidenceSink::default();
    let result = poll_immediate(invoker.invoke(
        workflow::AgentInvocationRequest {
            context: request_context(TENANT),
            agent_id: agent::AgentId::new(AGENT).expect("agent"),
            input: "{}".to_owned(),
            attempt_id: logical_id("attempt"),
            effective_capability_ceiling: agent::EffectiveCapabilityCeilingV1 {
                allowed_tool_ids: vec![],
                memory_enabled: false,
                knowledge_enabled: false,
                sandbox_execution_allowed: false,
                communication_allowed: false,
            },
            policy_decision_digest: "0".repeat(64),
        },
        llm_gateway::InvocationControl {
            idempotency_key: &key,
            cancellation: &cancellation,
            deadline: &deadline,
        },
        &mut evidence,
    ))
    .expect("static invocation");
    assert_eq!(result.capability_scope_digest.len(), 64);
    assert_eq!(evidence.0.len(), 1);
    assert_eq!(counts.tools.load(Ordering::Relaxed), 0);
    assert_eq!(counts.memory.load(Ordering::Relaxed), 0);
    assert_eq!(counts.knowledge.load(Ordering::Relaxed), 0);
    assert_eq!(counts.sandbox.load(Ordering::Relaxed), 0);
}

fn evaluated_result() -> evaluation::EvaluationResultV1 {
    let (store, summary) = completed_workflow();
    poll_immediate(
        EvaluationService::new(
            StrictEvidenceReader(store),
            evaluation::memory::InMemoryEvaluationStore::new(),
            evaluation::local::DeterministicCriteriaEvaluator,
        )
        .evaluate(TENANT, summary.id.as_str(), &evaluation_definition()),
    )
    .expect("evaluation")
}

#[test]
fn telemetry_is_tenant_scoped_fixed_metadata_only_and_obeys_capacity_and_query() {
    let local = observability::local::LocalTelemetry::new(1).expect("local telemetry");
    let first = evaluated_result();
    emit_telemetry(&local, &first).expect("first event");
    let mut second = first.clone();
    second.logical_key.workflow_run_id = "newer-run".to_owned();
    emit_telemetry(&local, &second).expect("second event evicts first");

    let query = TelemetryQueryV1::new(1).expect("query");
    let records = local
        .query(&TenantId::new(TENANT).expect("tenant"), &query)
        .expect("tenant query");
    assert_eq!(records.len(), 1);
    let record = &records[0];
    assert_eq!(record.envelope.timestamp.as_unix_nanos(), 1);
    assert_eq!(record.envelope.event.name.as_str(), "workflow_evaluated");
    assert_eq!(
        record.envelope.event.target.as_str(),
        "workflow_evaluation_demo"
    );
    assert_eq!(record.envelope.event.severity, Severity::Info);
    assert_eq!(record.envelope.event.body, "workflow evaluation completed");
    assert_eq!(
        record
            .envelope
            .event
            .attributes
            .keys()
            .map(String::as_str)
            .collect::<Vec<_>>(),
        [
            "evidence_digest",
            "result_digest",
            "verdict",
            "workflow_run_id"
        ]
    );
    assert_eq!(
        record.envelope.event.attributes["workflow_run_id"],
        "newer-run"
    );
    assert_eq!(record.envelope.event.attributes["verdict"], "pass");
    assert!(
        local
            .query(&TenantId::new("other-tenant").expect("tenant"), &query)
            .expect("foreign query")
            .is_empty()
    );

    let service = TelemetryService::new(local.clone(), local, FixedClock, 1).expect("service");
    let mut filtered = TelemetryQueryV1::new(1).expect("query");
    filtered.event_name = Some(EventName::new("different_event").expect("name"));
    assert!(
        service
            .query(
                &TelemetryContext::new(TenantId::new(TENANT).expect("tenant")),
                &filtered,
            )
            .expect("filtered query")
            .is_empty()
    );
}

#[test]
fn local_store_guarantees_truthfully_report_process_local_non_durable_behavior() {
    let evaluation =
        evaluation::memory::InMemoryEvaluationStore::with_capacities(1, 2).expect("bounded store");
    let guarantees = evaluation.guarantees();
    assert!(!guarantees.durable_across_restart);
    assert!(!guarantees.visible_across_processes);
    assert!(!guarantees.crash_atomic);
    assert!(!guarantees.evicts_on_capacity);
    assert_eq!(guarantees.max_results_per_tenant, 1);
    assert_eq!(guarantees.max_results_global, 2);

    let local = observability::local::LocalTelemetry::new(1).expect("local telemetry");
    let telemetry = TelemetryService::new(local.clone(), local, FixedClock, 1).expect("service");
    let guarantees = telemetry.guarantees();
    assert!(!guarantees.durable_across_restart);
    assert!(!guarantees.visible_across_processes);
    assert!(guarantees.delivery_confirmed);
    assert!(guarantees.queryable);
}
