use super::*;
use crate::{
    AgentStep, AttemptStatus, INVOCATION_TIMEOUT, InvocationPolicy, MAX_EVENTS, RequestContext,
    RunStatus, TerminalReason, WorkflowBudget, WorkflowRunner, WorkflowVersion,
};
use agent::EffectiveCapabilityCeilingV1;
use std::future::{Future, poll_fn};
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

fn id(value: &str) -> LogicalId {
    LogicalId::new(value).expect("logical id")
}

fn context(tenant: &str) -> RequestContext {
    RequestContext {
        tenant_id: id(tenant),
        principal_id: id("principal"),
        request_id: id("request"),
        correlation_id: id("correlation"),
    }
}

fn definition(max_evidence_bytes: usize) -> WorkflowDefinitionV1 {
    WorkflowDefinitionV1 {
        id: id("workflow"),
        version: WorkflowVersion::V1,
        step: AgentStep {
            agent_id: AgentId::new("agent").expect("agent"),
        },
        budget: WorkflowBudget {
            max_evidence_bytes,
            ..WorkflowBudget::default()
        },
    }
}

fn ceiling() -> EffectiveCapabilityCeilingV1 {
    EffectiveCapabilityCeilingV1 {
        allowed_tool_ids: vec!["read".to_owned()],
        memory_enabled: true,
        knowledge_enabled: false,
        sandbox_execution_allowed: false,
        communication_allowed: false,
    }
}

#[derive(Default)]
struct DeadlineState {
    elapsed: AtomicBool,
    waiter: Mutex<Option<Waker>>,
}

#[derive(Clone)]
struct TestDeadline {
    instant: Instant,
    state: Arc<DeadlineState>,
}
impl TestDeadline {
    fn elapse(&self) {
        self.state.elapsed.store(true, Ordering::Release);
        if let Some(waker) = self.state.waiter.lock().expect("waiter").take() {
            waker.wake();
        }
    }
}
impl llm_gateway::DeadlineSignal for TestDeadline {
    fn instant(&self) -> Instant {
        self.instant
    }
    fn is_elapsed(&self) -> bool {
        self.state.elapsed.load(Ordering::Acquire)
    }
    fn elapsed(&self) -> llm_gateway::DeadlineFuture<'_> {
        Box::pin(poll_fn(|cx| {
            if self.is_elapsed() {
                return Poll::Ready(());
            }
            *self.state.waiter.lock().expect("waiter") = Some(cx.waker().clone());
            if self.is_elapsed() {
                Poll::Ready(())
            } else {
                Poll::Pending
            }
        }))
    }
}

#[derive(Clone, Default)]
struct TestDeadlines(Arc<Mutex<Vec<TestDeadline>>>);
impl TestDeadlines {
    fn created(&self) -> Vec<TestDeadline> {
        self.0.lock().expect("deadlines").clone()
    }
}
impl llm_gateway::DeadlineFactory for TestDeadlines {
    fn create(&self, instant: Instant) -> Box<dyn llm_gateway::DeadlineSignal> {
        let deadline = TestDeadline {
            instant,
            state: Arc::new(DeadlineState::default()),
        };
        self.0.lock().expect("deadlines").push(deadline.clone());
        Box::new(deadline)
    }
}

#[derive(Clone, Copy)]
enum Behavior {
    Succeed,
    Fail,
    WaitCancellation,
    WaitDeadline,
    WaitGateThenSucceed,
}

#[derive(Clone)]
struct RecordingInvoker {
    behavior: Behavior,
    calls: Arc<AtomicUsize>,
    controls: Arc<Mutex<Vec<(String, Instant, bool)>>>,
    requests: Arc<Mutex<Vec<AgentInvocationRequest>>>,
    gate: Arc<AtomicBool>,
    gate_waker: Arc<Mutex<Option<Waker>>>,
}
impl RecordingInvoker {
    fn new(behavior: Behavior) -> Self {
        Self {
            behavior,
            calls: Arc::new(AtomicUsize::new(0)),
            controls: Arc::new(Mutex::new(Vec::new())),
            requests: Arc::new(Mutex::new(Vec::new())),
            gate: Arc::new(AtomicBool::new(false)),
            gate_waker: Arc::new(Mutex::new(None)),
        }
    }
    fn release(&self) {
        self.gate.store(true, Ordering::Release);
        if let Some(waker) = self.gate_waker.lock().expect("gate waker").take() {
            waker.wake();
        }
    }
}
impl AgentInvoker for RecordingInvoker {
    fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError> {
        Ok(id.as_str() == "agent")
    }

    fn invoke<'a>(
        &'a self,
        request: AgentInvocationRequest,
        control: llm_gateway::InvocationControl<'a>,
        evidence: &'a mut dyn InvocationEvidenceSink,
    ) -> crate::AgentInvocationFuture<'a> {
        self.calls.fetch_add(1, Ordering::Relaxed);
        self.controls.lock().expect("controls").push((
            control.idempotency_key.as_str().to_owned(),
            control.deadline.instant(),
            control.cancellation.is_cancelled(),
        ));
        self.requests.lock().expect("requests").push(request);
        Box::pin(async move {
            match self.behavior {
                Behavior::Succeed => {
                    evidence.emit(InvocationEvidence::new("result", "output")?)?;
                    Ok(AgentInvocationResult {
                        capability_scope_digest: "scope".to_owned(),
                    })
                }
                Behavior::Fail => Err(WorkflowError::AdapterFailure),
                Behavior::WaitCancellation => {
                    control.cancellation.cancelled().await;
                    Err(WorkflowError::Cancelled)
                }
                Behavior::WaitDeadline => {
                    control.deadline.elapsed().await;
                    Err(WorkflowError::DeadlineExceeded)
                }
                Behavior::WaitGateThenSucceed => {
                    poll_fn(|cx| {
                        if self.gate.load(Ordering::Acquire) {
                            Poll::Ready(())
                        } else {
                            *self.gate_waker.lock().expect("gate waker") = Some(cx.waker().clone());
                            Poll::Pending
                        }
                    })
                    .await;
                    evidence.emit(InvocationEvidence::new("result", "late")?)?;
                    Ok(AgentInvocationResult {
                        capability_scope_digest: "scope".to_owned(),
                    })
                }
            }
        })
    }
}

fn runner(
    store: InMemoryWorkflowStore,
    invoker: RecordingInvoker,
    deadlines: TestDeadlines,
    max_evidence_bytes: usize,
) -> WorkflowRunner<InMemoryWorkflowStore, StaticWorkflowCatalog, RecordingInvoker> {
    WorkflowRunner::new(
        store,
        StaticWorkflowCatalog::new([definition(max_evidence_bytes)]),
        invoker,
        Box::new(deadlines),
    )
}

fn poll<T>(future: &mut Pin<Box<dyn Future<Output = T> + Send + '_>>) -> Poll<T> {
    future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
}

fn ready<T>(mut future: Pin<Box<dyn Future<Output = T> + Send + '_>>) -> T {
    match poll(&mut future) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("deterministic future unexpectedly pending"),
    }
}

#[test]
fn duplicate_start_replays_without_invocation_and_conflicting_key_is_pre_effect() {
    let store = InMemoryWorkflowStore::default();
    let invoker = RecordingInvoker::new(Behavior::Succeed);
    let deadlines = TestDeadlines::default();
    let service = runner(
        store,
        invoker.clone(),
        deadlines.clone(),
        crate::MAX_EVIDENCE_BYTES,
    );

    let first = ready(service.start(
        context("tenant"),
        id("workflow"),
        WorkflowVersion::V1,
        "key".to_owned(),
        r#"{"b":2,"a":1}"#.to_owned(),
    ))
    .expect("first start");
    let replay = ready(service.start(
        context("tenant"),
        id("workflow"),
        WorkflowVersion::V1,
        "key".to_owned(),
        r#"{"a":1,"b":2}"#.to_owned(),
    ))
    .expect("replay");
    assert_eq!(first, replay);
    assert_eq!(invoker.calls.load(Ordering::Relaxed), 1);
    assert_eq!(deadlines.created().len(), 1);

    assert_eq!(
        ready(service.start(
            context("tenant"),
            id("workflow"),
            WorkflowVersion::V1,
            "key".to_owned(),
            r#"{"a":3}"#.to_owned(),
        )),
        Err(WorkflowError::RunKeyConflict)
    );
    assert_eq!(invoker.calls.load(Ordering::Relaxed), 1);
    assert_eq!(deadlines.created().len(), 1);
}

#[test]
fn invocation_control_is_stable_bounded_and_policy_is_forwarded_unchanged() {
    let invoker = RecordingInvoker::new(Behavior::Succeed);
    let deadlines = TestDeadlines::default();
    let service = runner(
        InMemoryWorkflowStore::default(),
        invoker.clone(),
        deadlines.clone(),
        crate::MAX_EVIDENCE_BYTES,
    );
    let before = Instant::now()
        + INVOCATION_TIMEOUT
            .checked_sub(Duration::from_secs(1))
            .expect("timeout exceeds one second");
    let policy = InvocationPolicy {
        effective_capability_ceiling: ceiling(),
        policy_decision_digest: "a".repeat(64),
    };
    ready(service.start_with_policy(
        context("tenant"),
        id("workflow"),
        WorkflowVersion::V1,
        "key".to_owned(),
        "{}".to_owned(),
        policy.clone(),
    ))
    .expect("start");

    let controls = invoker.controls.lock().expect("controls");
    assert_eq!(controls.len(), 1);
    assert_eq!(controls[0].0.len(), 64);
    assert!(controls[0].0.bytes().all(|byte| byte.is_ascii_hexdigit()));
    assert!(!controls[0].2);
    assert!(controls[0].1 >= before);
    assert_eq!(controls[0].1, deadlines.created()[0].instant);
    let requests = invoker.requests.lock().expect("requests");
    assert_eq!(
        requests[0].effective_capability_ceiling,
        policy.effective_capability_ceiling
    );
    assert_eq!(
        requests[0].policy_decision_digest,
        policy.policy_decision_digest
    );
}

#[test]
fn invalid_inputs_are_rejected_before_invocation_or_deadline_creation() {
    let invoker = RecordingInvoker::new(Behavior::Succeed);
    let deadlines = TestDeadlines::default();
    let service = runner(
        InMemoryWorkflowStore::default(),
        invoker.clone(),
        deadlines.clone(),
        crate::MAX_EVIDENCE_BYTES,
    );
    for (run_key, input) in [
        (String::new(), "{}".to_owned()),
        ("key".to_owned(), "not-json".to_owned()),
    ] {
        assert!(
            ready(service.start(
                context("tenant"),
                id("workflow"),
                WorkflowVersion::V1,
                run_key,
                input,
            ))
            .is_err()
        );
    }
    assert_eq!(invoker.calls.load(Ordering::Relaxed), 0);
    assert!(deadlines.created().is_empty());
}

#[test]
fn failed_invocation_is_not_retried() {
    let store = InMemoryWorkflowStore::default();
    let invoker = RecordingInvoker::new(Behavior::Fail);
    let service = runner(
        store.clone(),
        invoker.clone(),
        TestDeadlines::default(),
        crate::MAX_EVIDENCE_BYTES,
    );
    let summary = ready(service.start(
        context("tenant"),
        id("workflow"),
        WorkflowVersion::V1,
        "key".to_owned(),
        "{}".to_owned(),
    ))
    .expect("terminal summary");
    assert_eq!(summary.status, RunStatus::Failed);
    assert_eq!(invoker.calls.load(Ordering::Relaxed), 1);
    assert_eq!(store.list(&id("tenant")).expect("list").len(), 1);
}

#[test]
fn tenant_scoping_hides_get_list_and_transition() {
    let store = InMemoryWorkflowStore::default();
    let service = runner(
        store.clone(),
        RecordingInvoker::new(Behavior::Succeed),
        TestDeadlines::default(),
        crate::MAX_EVIDENCE_BYTES,
    );
    let summary = ready(service.start(
        context("tenant-a"),
        id("workflow"),
        WorkflowVersion::V1,
        "key".to_owned(),
        "{}".to_owned(),
    ))
    .expect("start");
    assert_eq!(
        service.get(&id("tenant-b"), summary.id.clone()),
        Err(WorkflowError::NotFound)
    );
    assert!(service.list(&id("tenant-b")).expect("list").is_empty());
    assert_eq!(
        store
            .transition(
                &id("tenant-b"),
                &summary.id,
                summary.revision,
                RunStatus::Succeeded,
                Transition {
                    status: RunStatus::Failed,
                    terminal_reason: Some(TerminalReason::InvocationFailed),
                    attempt: None,
                    events: vec![],
                },
            )
            .expect("transition"),
        TransitionResult::NotFound
    );
}

#[test]
fn cancellation_wakes_pending_start_and_wins_terminal_cas() {
    let store = InMemoryWorkflowStore::default();
    let service = runner(
        store.clone(),
        RecordingInvoker::new(Behavior::WaitCancellation),
        TestDeadlines::default(),
        crate::MAX_EVIDENCE_BYTES,
    );
    let mut start = service.start(
        context("tenant"),
        id("workflow"),
        WorkflowVersion::V1,
        "key".to_owned(),
        "{}".to_owned(),
    );
    assert!(poll(&mut start).is_pending());
    let running = store.list(&id("tenant")).expect("list").pop().expect("run");
    let cancelled = service
        .cancel(&id("tenant"), running.id.clone())
        .expect("cancel");
    assert_eq!(cancelled.status, RunStatus::Cancelled);
    let completed = match poll(&mut start) {
        Poll::Ready(value) => value.expect("summary"),
        Poll::Pending => panic!("cancellation did not wake start"),
    };
    assert_eq!(completed.status, RunStatus::Cancelled);
    assert_eq!(
        store
            .get(&id("tenant"), &running.id)
            .expect("get")
            .expect("run")
            .events
            .len(),
        2
    );
}

#[test]
fn deadline_wakes_pending_start_and_terminalizes_failure() {
    let store = InMemoryWorkflowStore::default();
    let deadlines = TestDeadlines::default();
    let service = runner(
        store.clone(),
        RecordingInvoker::new(Behavior::WaitDeadline),
        deadlines.clone(),
        crate::MAX_EVIDENCE_BYTES,
    );
    let mut start = service.start(
        context("tenant"),
        id("workflow"),
        WorkflowVersion::V1,
        "key".to_owned(),
        "{}".to_owned(),
    );
    assert!(poll(&mut start).is_pending());
    deadlines.created()[0].elapse();
    let summary = match poll(&mut start) {
        Poll::Ready(value) => value.expect("summary"),
        Poll::Pending => panic!("deadline did not wake start"),
    };
    assert_eq!(summary.status, RunStatus::Failed);
    let run = store.list(&id("tenant")).expect("list").pop().expect("run");
    assert_eq!(
        run.attempt.expect("attempt").error.as_deref(),
        Some("deadline_exceeded")
    );
}

#[test]
fn late_success_cannot_overwrite_cancellation_or_persist_result() {
    let store = InMemoryWorkflowStore::default();
    let invoker = RecordingInvoker::new(Behavior::WaitGateThenSucceed);
    let service = runner(
        store.clone(),
        invoker.clone(),
        TestDeadlines::default(),
        crate::MAX_EVIDENCE_BYTES,
    );
    let mut start = service.start(
        context("tenant"),
        id("workflow"),
        WorkflowVersion::V1,
        "key".to_owned(),
        "{}".to_owned(),
    );
    assert!(poll(&mut start).is_pending());
    let running = store.list(&id("tenant")).expect("list").pop().expect("run");
    service
        .cancel(&id("tenant"), running.id.clone())
        .expect("cancel");
    invoker.release();
    let summary = match poll(&mut start) {
        Poll::Ready(value) => value.expect("summary"),
        Poll::Pending => panic!("gate did not wake start"),
    };
    assert_eq!(summary.status, RunStatus::Cancelled);
    let run = store
        .get(&id("tenant"), &running.id)
        .expect("get")
        .expect("run");
    assert_eq!(
        run.events
            .iter()
            .map(|event| event.kind.as_str())
            .collect::<Vec<_>>(),
        ["started", "cancelled"]
    );
    assert!(run.attempt.expect("attempt").result.is_none());
}

#[test]
fn unusable_evidence_budget_is_rejected_before_creating_a_run() {
    let store = InMemoryWorkflowStore::default();
    let invoker = RecordingInvoker::new(Behavior::Succeed);
    let deadlines = TestDeadlines::default();
    let service = runner(store.clone(), invoker.clone(), deadlines.clone(), 1);
    assert_eq!(
        ready(service.start(
            context("tenant"),
            id("workflow"),
            WorkflowVersion::V1,
            "key".to_owned(),
            "{}".to_owned(),
        )),
        Err(WorkflowError::InvalidDefinition)
    );
    assert!(store.list(&id("tenant")).expect("list").is_empty());
    assert_eq!(invoker.calls.load(Ordering::Relaxed), 0);
    assert!(deadlines.created().is_empty());
}

#[test]
fn evidence_overflow_persists_no_partial_invocation_evidence() {
    let store = InMemoryWorkflowStore::default();
    let mut items = vec![InvocationEvidence::new("event", "x").expect("event"); MAX_EVENTS - 1];
    items.push(InvocationEvidence::new("result", "x").expect("result"));
    let service = WorkflowRunner::new(
        store.clone(),
        StaticWorkflowCatalog::new([definition(crate::MAX_EVIDENCE_BYTES)]),
        StaticAgentInvoker::new(
            vec![AgentId::new("agent").expect("agent")],
            StaticInvocation::Succeed {
                capability_scope_digest: "scope".to_owned(),
                evidence: items,
            },
        ),
        Box::new(TestDeadlines::default()),
    );
    assert_eq!(
        ready(service.start(
            context("tenant"),
            id("workflow"),
            WorkflowVersion::V1,
            "key".to_owned(),
            "{}".to_owned(),
        )),
        Err(WorkflowError::LimitExceeded)
    );
    let run = store.list(&id("tenant")).expect("list").pop().expect("run");
    assert_eq!(
        run.events
            .iter()
            .map(|event| event.kind.as_str())
            .collect::<Vec<_>>(),
        ["started", "invocation_failed"]
    );
}

#[test]
fn dropped_start_unregisters_cancellation_and_cancel_requires_active_registration() {
    let store = InMemoryWorkflowStore::default();
    let service = runner(
        store.clone(),
        RecordingInvoker::new(Behavior::WaitCancellation),
        TestDeadlines::default(),
        crate::MAX_EVIDENCE_BYTES,
    );
    let mut start = service.start(
        context("tenant"),
        id("workflow"),
        WorkflowVersion::V1,
        "key".to_owned(),
        "{}".to_owned(),
    );
    assert!(poll(&mut start).is_pending());
    let running = store.list(&id("tenant")).expect("list").pop().expect("run");
    drop(start);
    assert!(
        service
            .cancellations
            .lock()
            .expect("cancellations")
            .is_empty()
    );
    assert_eq!(
        service.cancel(&id("tenant"), running.id),
        Err(WorkflowError::Conflict)
    );
}

#[test]
fn registration_token_guard_does_not_remove_a_newer_registration() {
    let registrations = Mutex::new(BTreeMap::new());
    let run_id = id("run");
    registrations.lock().expect("registrations").insert(
        run_id.clone(),
        crate::ActiveCancellation {
            token: 2,
            signal: crate::CancellationSignal::new(),
        },
    );
    let stale = crate::CancellationRegistration {
        registrations: &registrations,
        run_id: run_id.clone(),
        token: 1,
    };
    drop(stale);
    assert_eq!(
        registrations
            .lock()
            .expect("registrations")
            .get(&run_id)
            .expect("active")
            .token,
        2
    );
}

#[test]
fn poisoned_store_fails_closed_without_leaking_state() {
    let store = InMemoryWorkflowStore::default();
    let state = Arc::clone(&store.state);
    let _ = std::thread::spawn(move || {
        let _guard = state.lock().expect("state");
        panic!("poison state");
    })
    .join();
    assert_eq!(
        store.list(&id("tenant")),
        Err(WorkflowError::AdapterFailure)
    );
    assert_eq!(
        store.get(&id("tenant"), &id("run")),
        Err(WorkflowError::AdapterFailure)
    );
}

#[test]
fn static_invoker_preflight_prevents_evidence_on_cancelled_control() {
    struct NeverDeadline;
    impl llm_gateway::DeadlineSignal for NeverDeadline {
        fn instant(&self) -> Instant {
            Instant::now() + Duration::from_secs(1)
        }
        fn is_elapsed(&self) -> bool {
            false
        }
        fn elapsed(&self) -> llm_gateway::DeadlineFuture<'_> {
            Box::pin(std::future::pending())
        }
    }
    #[derive(Default)]
    struct Sink(Vec<InvocationEvidence>);
    impl InvocationEvidenceSink for Sink {
        fn emit(&mut self, evidence: InvocationEvidence) -> Result<(), WorkflowError> {
            self.0.push(evidence);
            Ok(())
        }
    }
    let invoker = StaticAgentInvoker::new(
        vec![AgentId::new("agent").expect("agent")],
        StaticInvocation::Succeed {
            capability_scope_digest: "scope".to_owned(),
            evidence: vec![InvocationEvidence::new("result", "secret").expect("result")],
        },
    );
    let signal = crate::CancellationSignal::new();
    signal.cancel();
    let key = llm_gateway::IdempotencyKey::new("key").expect("key");
    let deadline = NeverDeadline;
    let mut sink = Sink::default();
    let result = ready(invoker.invoke(
        AgentInvocationRequest {
            context: context("tenant"),
            agent_id: AgentId::new("agent").expect("agent"),
            input: "{}".to_owned(),
            attempt_id: id("attempt"),
            effective_capability_ceiling: ceiling(),
            policy_decision_digest: "a".repeat(64),
        },
        llm_gateway::InvocationControl {
            idempotency_key: &key,
            cancellation: &signal,
            deadline: &deadline,
        },
        &mut sink,
    ));
    assert_eq!(result, Err(WorkflowError::Cancelled));
    assert!(sink.0.is_empty());
}

#[test]
fn successful_attempt_preserves_policy_and_result_in_store() {
    let store = InMemoryWorkflowStore::default();
    let service = runner(
        store.clone(),
        RecordingInvoker::new(Behavior::Succeed),
        TestDeadlines::default(),
        crate::MAX_EVIDENCE_BYTES,
    );
    let policy = InvocationPolicy {
        effective_capability_ceiling: ceiling(),
        policy_decision_digest: "b".repeat(64),
    };
    let summary = ready(service.start_with_policy(
        context("tenant"),
        id("workflow"),
        WorkflowVersion::V1,
        "key".to_owned(),
        "{}".to_owned(),
        policy.clone(),
    ))
    .expect("start");
    let attempt = store
        .get(&id("tenant"), &summary.id)
        .expect("get")
        .expect("run")
        .attempt
        .expect("attempt");
    assert_eq!(attempt.status, AttemptStatus::Succeeded);
    assert_eq!(
        attempt.effective_capability_ceiling,
        policy.effective_capability_ceiling
    );
    assert_eq!(
        attempt.policy_decision_digest,
        policy.policy_decision_digest
    );
    assert_eq!(attempt.result.as_deref(), Some("output"));
}

#[test]
fn poisoned_cancellation_registry_terminalizes_created_attempt() {
    let store = InMemoryWorkflowStore::default();
    let service = runner(
        store.clone(),
        RecordingInvoker::new(Behavior::Succeed),
        TestDeadlines::default(),
        crate::MAX_EVIDENCE_BYTES,
    );
    let _ = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let _guard = service.cancellations.lock().expect("cancellations");
        panic!("poison cancellations");
    }));
    assert_eq!(
        ready(service.start(
            context("tenant"),
            id("workflow"),
            WorkflowVersion::V1,
            "key".to_owned(),
            "{}".to_owned(),
        )),
        Err(WorkflowError::AdapterFailure)
    );
    let run = store.list(&id("tenant")).expect("list").pop().expect("run");
    assert_eq!(run.status, RunStatus::Failed);
    assert_eq!(
        run.attempt.expect("attempt").error.as_deref(),
        Some("cancellation_registration_failed")
    );
}

#[test]
fn registration_token_overflow_terminalizes_without_invocation() {
    let store = InMemoryWorkflowStore::default();
    let invoker = RecordingInvoker::new(Behavior::Succeed);
    let service = runner(
        store.clone(),
        invoker.clone(),
        TestDeadlines::default(),
        crate::MAX_EVIDENCE_BYTES,
    );
    service.next_registration.store(u64::MAX, Ordering::Relaxed);
    assert_eq!(
        ready(service.start(
            context("tenant"),
            id("workflow"),
            WorkflowVersion::V1,
            "key".to_owned(),
            "{}".to_owned(),
        )),
        Err(WorkflowError::AdapterFailure)
    );
    assert_eq!(invoker.calls.load(Ordering::Relaxed), 0);
    assert_eq!(
        store.list(&id("tenant")).expect("list")[0].status,
        RunStatus::Failed
    );
}
