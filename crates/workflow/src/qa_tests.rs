use super::*;
use llm_gateway::{CancellationHandle, CancellationSignalFactory};
use std::sync::atomic::{AtomicBool, Ordering as AtomicOrdering};
use std::task::Waker;

struct ObjectSafeInvoker;
impl AgentInvoker for ObjectSafeInvoker {
    fn validate_agent(&self, _: &AgentId) -> Result<bool, WorkflowError> {
        Ok(true)
    }
    fn invoke<'a>(
        &'a self,
        _: AgentInvocationRequest,
        _: llm_gateway::InvocationControl<'a>,
        _: &'a mut dyn InvocationEvidenceSink,
    ) -> AgentInvocationFuture<'a> {
        Box::pin(async {
            Ok(AgentInvocationResult {
                capability_scope_digest: "scope".to_owned(),
            })
        })
    }
}

struct Deadline(Instant);
impl llm_gateway::DeadlineSignal for Deadline {
    fn instant(&self) -> Instant {
        self.0
    }
    fn is_elapsed(&self) -> bool {
        false
    }
    fn elapsed(&self) -> llm_gateway::DeadlineFuture<'_> {
        Box::pin(std::future::pending())
    }
}
struct Factory;
impl llm_gateway::DeadlineFactory for Factory {
    fn create(&self, instant: Instant) -> Box<dyn llm_gateway::DeadlineSignal> {
        Box::new(Deadline(instant))
    }
}

/// Trivial test-only `CancellationHandle`/`CancellationSignalFactory`
/// fixture. Unlike the deleted hand-rolled `WaiterRegistry`, this supports
/// exactly one waiter (matching the SME's finding that real usage is
/// exactly one `.cancelled()` future polled per in-flight attempt) rather
/// than a bounded broadcast registry — no capacity limit, no token space,
/// no poisoning: those were artifacts of the deleted implementation, not
/// product requirements (see #72).
#[derive(Clone, Default)]
pub(crate) struct TestCancellationHandle(Arc<TestCancellationState>);
#[derive(Default)]
struct TestCancellationState {
    cancelled: AtomicBool,
    waker: Mutex<Option<Waker>>,
}
impl llm_gateway::CancellationSignal for TestCancellationHandle {
    fn is_cancelled(&self) -> bool {
        self.0.cancelled.load(AtomicOrdering::Acquire)
    }
    fn cancelled(&self) -> llm_gateway::CancellationFuture<'_> {
        Box::pin(TestCancellationWait(&self.0))
    }
}
impl llm_gateway::CancellationHandle for TestCancellationHandle {
    fn cancel(&self) {
        self.0.cancelled.store(true, AtomicOrdering::Release);
        if let Some(waker) = self.0.waker.lock().expect("waker").take() {
            waker.wake();
        }
    }
}
struct TestCancellationWait<'a>(&'a TestCancellationState);
impl std::future::Future for TestCancellationWait<'_> {
    type Output = ();
    fn poll(
        self: std::pin::Pin<&mut Self>,
        context: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        // Register the waker before re-checking `cancelled`, not after: if
        // the order were reversed, a `cancel()` landing between the check
        // and the registration would see no waker to wake, and this poll
        // would then install a waker for an event that already fired,
        // waiting forever. Registering first closes that race - whichever
        // side (this check, or `cancel()`'s take()) runs second observes
        // the other's effect.
        *self.0.waker.lock().expect("waker") = Some(context.waker().clone());
        if self.0.cancelled.load(AtomicOrdering::Acquire) {
            return std::task::Poll::Ready(());
        }
        std::task::Poll::Pending
    }
}
#[derive(Clone, Copy, Default)]
pub(crate) struct TestCancellationSignalFactory;
impl llm_gateway::CancellationSignalFactory for TestCancellationSignalFactory {
    fn create(&self) -> Arc<dyn llm_gateway::CancellationHandle> {
        Arc::new(TestCancellationHandle::default())
    }
}

#[test]
fn async_workflow_ports_are_object_safe() {
    let invoker: &dyn AgentInvoker = &ObjectSafeInvoker;
    let factory: &dyn llm_gateway::DeadlineFactory = &Factory;
    let signal = TestCancellationSignalFactory.create();
    let cancellation: &dyn llm_gateway::CancellationSignal = signal.as_ref();
    let deadline = factory.create(Instant::now());
    let deadline_signal: &dyn llm_gateway::DeadlineSignal = deadline.as_ref();
    assert!(
        invoker
            .validate_agent(&AgentId::new("agent").expect("agent"))
            .expect("validation")
    );
    assert!(!cancellation.is_cancelled());
    assert_eq!(deadline_signal.instant(), deadline.instant());
}

#[test]
fn cancel_before_wait_completes_immediately() {
    let signal = TestCancellationSignalFactory.create();
    signal.cancel();
    let mut wait = llm_gateway::CancellationSignal::cancelled(signal.as_ref());
    let waker = Waker::noop();
    assert!(
        wait.as_mut()
            .poll(&mut std::task::Context::from_waker(waker))
            .is_ready()
    );
}

#[test]
fn a_pending_wait_reflects_state_after_cancel() {
    let signal = TestCancellationSignalFactory.create();
    assert!(!signal.is_cancelled());
    signal.cancel();
    assert!(signal.is_cancelled());
}

#[test]
fn cancel_is_idempotent() {
    let signal = TestCancellationSignalFactory.create();
    signal.cancel();
    signal.cancel();
    assert!(signal.is_cancelled());
}

#[test]
fn cloned_handles_share_the_same_underlying_state() {
    let signal: Arc<dyn CancellationHandle> = TestCancellationSignalFactory.create();
    let clone = Arc::clone(&signal);
    clone.cancel();
    assert!(signal.is_cancelled());
}

struct WakeCount(std::sync::atomic::AtomicUsize);
impl std::task::Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, AtomicOrdering::Relaxed);
    }
}

#[test]
fn cancellation_between_registration_and_drain_has_no_lost_wake() {
    // Regression guard for the exact race qa-tester caught: cancel()
    // landing between the fixture's is_cancelled() check and its waker
    // registration must not leave a pending poll waiting forever.
    let signal = TestCancellationSignalFactory.create();
    let mut wait = signal.cancelled();
    let count = Arc::new(WakeCount(std::sync::atomic::AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&count));
    assert!(
        wait.as_mut()
            .poll(&mut std::task::Context::from_waker(&waker))
            .is_pending()
    );

    let cancel_signal = Arc::clone(&signal);
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let cancel = std::thread::spawn(move || {
        started_tx.send(()).expect("start notification");
        cancel_signal.cancel();
    });
    started_rx.recv().expect("cancel started");
    while !signal.is_cancelled() {
        std::thread::yield_now();
    }
    cancel.join().expect("cancel thread");

    assert_eq!(count.0.load(AtomicOrdering::Relaxed), 1);
    assert!(
        wait.as_mut()
            .poll(&mut std::task::Context::from_waker(&waker))
            .is_ready()
    );
}

#[test]
fn two_signals_from_the_same_factory_are_independent() {
    let factory = TestCancellationSignalFactory;
    let first = factory.create();
    let second = factory.create();
    first.cancel();
    assert!(first.is_cancelled());
    assert!(!second.is_cancelled());
}

#[test]
fn evidence_chunk_accepts_exact_limit_and_rejects_one_over_and_empty_kind() {
    let kind = "k";
    assert!(InvocationEvidence::new(kind, "x".repeat(MAX_EVIDENCE_CHUNK_BYTES - 1)).is_ok());
    assert_eq!(
        InvocationEvidence::new(kind, "x".repeat(MAX_EVIDENCE_CHUNK_BYTES)),
        Err(WorkflowError::LimitExceeded)
    );
    assert_eq!(
        InvocationEvidence::new("", "data"),
        Err(WorkflowError::LimitExceeded)
    );
}

#[test]
fn evidence_collector_checks_existing_bytes_count_and_duplicate_result() {
    let existing = vec![WorkflowEvent {
        sequence: 1,
        kind: "started".to_owned(),
        data: String::new(),
    }];
    let mut collector = EvidenceCollector::new(MAX_EVIDENCE_BYTES, &existing).expect("collector");
    collector
        .emit(InvocationEvidence::new("result", "one").expect("result"))
        .expect("first result");
    assert_eq!(
        collector.emit(InvocationEvidence::new("result", "two").expect("result")),
        Err(WorkflowError::InvalidRequest)
    );

    let too_many = (0..=MAX_EVENTS)
        .map(|index| WorkflowEvent {
            sequence: index as u64 + 1,
            kind: "e".to_owned(),
            data: String::new(),
        })
        .collect::<Vec<_>>();
    assert!(matches!(
        EvidenceCollector::new(MAX_EVIDENCE_BYTES, &too_many),
        Err(WorkflowError::LimitExceeded)
    ));
}

#[test]
fn invalid_transitions_cannot_mutate_terminal_runs_or_skip_sequences() {
    let attempt = Attempt {
        id: LogicalId::new("attempt").expect("attempt"),
        agent_id: AgentId::new("agent").expect("agent"),
        effective_capability_ceiling: EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: vec![],
            memory_enabled: false,
            knowledge_enabled: false,
            sandbox_execution_allowed: false,
            communication_allowed: false,
        },
        policy_decision_digest: "a".repeat(64),
        capability_scope_digest: None,
        status: AttemptStatus::Running,
        result: None,
        error: None,
    };
    let run = Run {
        id: LogicalId::new("run").expect("run"),
        context: RequestContext {
            tenant_id: LogicalId::new("tenant").expect("tenant"),
            principal_id: LogicalId::new("principal").expect("principal"),
            request_id: LogicalId::new("request").expect("request"),
            correlation_id: LogicalId::new("correlation").expect("correlation"),
        },
        workflow_id: LogicalId::new("workflow").expect("workflow"),
        workflow_version: WorkflowVersion::V1,
        run_key: "key".to_owned(),
        input_digest: "digest".to_owned(),
        max_evidence_bytes: MAX_EVIDENCE_BYTES,
        status: RunStatus::Running,
        revision: 1,
        terminal_reason: None,
        attempt: Some(attempt.clone()),
        events: vec![WorkflowEvent {
            sequence: 1,
            kind: "started".to_owned(),
            data: String::new(),
        }],
    };
    let mut failed = attempt;
    failed.status = AttemptStatus::Failed;
    failed.error = Some("failed".to_owned());
    let transition = Transition {
        status: RunStatus::Failed,
        terminal_reason: Some(TerminalReason::InvocationFailed),
        attempt: Some(failed),
        events: vec![WorkflowEvent {
            sequence: 3,
            kind: "invocation_failed".to_owned(),
            data: "failed".to_owned(),
        }],
    };
    assert!(!transition_is_valid(&run, RunStatus::Running, &transition));
}

#[test]
fn public_errors_are_closed_and_do_not_include_adapter_details() {
    for error in [
        WorkflowError::Cancelled,
        WorkflowError::DeadlineExceeded,
        WorkflowError::AdapterFailure,
    ] {
        assert_eq!(error.public_code(), PublicErrorCode::OperationFailed);
        let display = error.to_string();
        assert_eq!(display, "workflow operation failed: OperationFailed");
        assert!(!display.contains("secret"));
    }
}
