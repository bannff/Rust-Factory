use super::*;
use std::sync::atomic::{AtomicUsize, Ordering as AtomicOrdering};
use std::sync::{Weak, mpsc};
use std::task::{Context, Wake};
use std::time::Duration;

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

#[test]
fn async_workflow_ports_are_object_safe() {
    let invoker: &dyn AgentInvoker = &ObjectSafeInvoker;
    let factory: &dyn llm_gateway::DeadlineFactory = &Factory;
    let signal = CancellationSignal::new();
    let cancellation: &dyn llm_gateway::CancellationSignal = &signal;
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

struct WakeCount(AtomicUsize);
impl Wake for WakeCount {
    fn wake(self: Arc<Self>) {
        self.0.fetch_add(1, AtomicOrdering::Relaxed);
    }
}

#[test]
fn cancellation_wakes_all_waiters_and_all_become_ready() {
    let signal = CancellationSignal::new();
    let counts = (0..3)
        .map(|_| Arc::new(WakeCount(AtomicUsize::new(0))))
        .collect::<Vec<_>>();
    let wakers = counts
        .iter()
        .map(|count| Waker::from(Arc::clone(count)))
        .collect::<Vec<_>>();
    let mut waits = (0..3)
        .map(|_| llm_gateway::CancellationSignal::cancelled(&signal))
        .collect::<Vec<_>>();

    for (wait, waker) in waits.iter_mut().zip(&wakers) {
        assert!(
            wait.as_mut()
                .poll(&mut Context::from_waker(waker))
                .is_pending()
        );
    }
    signal.cancel();

    assert!(
        signal
            .0
            .registry
            .lock()
            .expect("registry")
            .waiters
            .is_empty()
    );
    for count in &counts {
        assert_eq!(count.0.load(AtomicOrdering::Relaxed), 1);
    }
    for (wait, waker) in waits.iter_mut().zip(&wakers) {
        assert!(
            wait.as_mut()
                .poll(&mut Context::from_waker(waker))
                .is_ready()
        );
        assert!(
            wait.as_mut()
                .poll(&mut Context::from_waker(waker))
                .is_ready()
        );
    }
}

#[test]
fn repeated_cancel_is_idempotent_and_new_waiters_are_stickily_ready() {
    let signal = CancellationSignal::new();
    let counts = (0..2)
        .map(|_| Arc::new(WakeCount(AtomicUsize::new(0))))
        .collect::<Vec<_>>();
    let wakers = counts
        .iter()
        .map(|count| Waker::from(Arc::clone(count)))
        .collect::<Vec<_>>();
    let mut waits = (0..2)
        .map(|_| llm_gateway::CancellationSignal::cancelled(&signal))
        .collect::<Vec<_>>();
    for (wait, waker) in waits.iter_mut().zip(&wakers) {
        assert!(
            wait.as_mut()
                .poll(&mut Context::from_waker(waker))
                .is_pending()
        );
    }

    signal.cancel();
    signal.cancel();

    assert!(
        signal
            .0
            .registry
            .lock()
            .expect("registry")
            .waiters
            .is_empty()
    );
    for count in &counts {
        assert_eq!(count.0.load(AtomicOrdering::Relaxed), 1);
    }
    for (wait, waker) in waits.iter_mut().zip(&wakers) {
        assert!(
            wait.as_mut()
                .poll(&mut Context::from_waker(waker))
                .is_ready()
        );
        assert!(
            wait.as_mut()
                .poll(&mut Context::from_waker(waker))
                .is_ready()
        );
    }

    let mut late = llm_gateway::CancellationSignal::cancelled(&signal);
    assert!(
        late.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );
    assert!(
        late.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );
    assert!(
        signal
            .0
            .registry
            .lock()
            .expect("registry")
            .waiters
            .is_empty()
    );
}

#[test]
fn changed_waker_updates_only_its_own_entry_without_growth() {
    let signal = CancellationSignal::new();
    let mut first = llm_gateway::CancellationSignal::cancelled(&signal);
    let mut second = llm_gateway::CancellationSignal::cancelled(&signal);
    let old_first_count = Arc::new(WakeCount(AtomicUsize::new(0)));
    let new_first_count = Arc::new(WakeCount(AtomicUsize::new(0)));
    let second_count = Arc::new(WakeCount(AtomicUsize::new(0)));
    let old_first_waker = Waker::from(Arc::clone(&old_first_count));
    let new_first_waker = Waker::from(Arc::clone(&new_first_count));
    let second_waker = Waker::from(Arc::clone(&second_count));

    assert!(
        first
            .as_mut()
            .poll(&mut Context::from_waker(&old_first_waker))
            .is_pending()
    );
    assert!(
        second
            .as_mut()
            .poll(&mut Context::from_waker(&second_waker))
            .is_pending()
    );
    assert!(
        first
            .as_mut()
            .poll(&mut Context::from_waker(&new_first_waker))
            .is_pending()
    );
    assert_eq!(signal.0.registry.lock().expect("registry").waiters.len(), 2);

    signal.cancel();
    assert_eq!(old_first_count.0.load(AtomicOrdering::Relaxed), 0);
    assert_eq!(new_first_count.0.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(second_count.0.load(AtomicOrdering::Relaxed), 1);
}

struct ReentrantDropWake {
    state: Weak<CancellationState>,
}
impl Wake for ReentrantDropWake {
    fn wake(self: Arc<Self>) {}
}
impl Drop for ReentrantDropWake {
    fn drop(&mut self) {
        if let Some(state) = self.state.upgrade() {
            CancellationSignal(state).cancel();
        }
    }
}

fn assert_reentrant_waker_drop_completes(replace: bool) {
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let signal = CancellationSignal::new();
        let mut wait = llm_gateway::CancellationSignal::cancelled(&signal);
        let wake = Arc::new(ReentrantDropWake {
            state: Arc::downgrade(&signal.0),
        });
        let waker = Waker::from(Arc::clone(&wake));
        assert!(
            wait.as_mut()
                .poll(&mut Context::from_waker(&waker))
                .is_pending()
        );
        drop(waker);
        drop(wake);

        if replace {
            assert!(
                wait.as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_pending()
            );
            assert!(signal.is_cancelled());
            assert!(
                wait.as_mut()
                    .poll(&mut Context::from_waker(Waker::noop()))
                    .is_ready()
            );
        } else {
            drop(wait);
            assert!(signal.is_cancelled());
        }
        done_tx.send(()).expect("completion notification");
    });

    done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("reentrant waker drop must not run while the registry is locked");
    worker.join().expect("waker-drop worker");
}

#[test]
fn replacement_and_removal_drop_reentrant_wakers_outside_registry_lock() {
    assert_reentrant_waker_drop_completes(true);
    assert_reentrant_waker_drop_completes(false);
}

#[test]
fn dropping_wait_unregisters_only_its_own_token() {
    let signal = CancellationSignal::new();
    let mut first = llm_gateway::CancellationSignal::cancelled(&signal);
    let mut second = llm_gateway::CancellationSignal::cancelled(&signal);
    assert!(
        first
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert!(
        second
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    assert_eq!(signal.0.registry.lock().expect("registry").waiters.len(), 2);

    drop(first);
    let registry = signal.0.registry.lock().expect("registry");
    assert_eq!(registry.waiters.len(), 1);
    assert!(registry.waiters.contains_key(&1));
    drop(registry);
    drop(second);
    assert!(
        signal
            .0
            .registry
            .lock()
            .expect("registry")
            .waiters
            .is_empty()
    );
}

#[test]
fn cancellation_capacity_accepts_exactly_64_and_65th_fails_closed() {
    let signal = CancellationSignal::new();
    let mut waits = (0..MAX_CANCELLATION_WAITERS)
        .map(|_| llm_gateway::CancellationSignal::cancelled(&signal))
        .collect::<Vec<_>>();
    for wait in &mut waits {
        assert!(
            wait.as_mut()
                .poll(&mut Context::from_waker(Waker::noop()))
                .is_pending()
        );
    }
    assert_eq!(
        signal.0.registry.lock().expect("registry").waiters.len(),
        64
    );

    let mut overflow = llm_gateway::CancellationSignal::cancelled(&signal);
    assert!(
        overflow
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );
    assert!(
        overflow
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );
    assert_eq!(
        signal.0.registry.lock().expect("registry").waiters.len(),
        64
    );
}

#[test]
fn cancellation_token_issues_u64_max_once_and_never_wraps() {
    let signal = CancellationSignal::new();
    signal.0.registry.lock().expect("registry").next_token = Some(u64::MAX);
    let mut last = llm_gateway::CancellationSignal::cancelled(&signal);
    assert!(
        last.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    {
        let registry = signal.0.registry.lock().expect("registry");
        assert!(registry.waiters.contains_key(&u64::MAX));
        assert_eq!(registry.next_token, None);
    }

    let mut exhausted = llm_gateway::CancellationSignal::cancelled(&signal);
    assert!(
        exhausted
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );
    assert!(
        exhausted
            .as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );
    assert_eq!(signal.0.registry.lock().expect("registry").waiters.len(), 1);
}

#[test]
fn cancellation_observed_before_poll_is_immediately_and_stickily_ready() {
    let signal = CancellationSignal::new();
    signal.cancel();
    let mut wait = llm_gateway::CancellationSignal::cancelled(&signal);
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );
    assert!(
        signal
            .0
            .registry
            .lock()
            .expect("registry")
            .waiters
            .is_empty()
    );
}

#[test]
fn cancellation_published_before_under_lock_check_fails_closed_without_admission() {
    let signal = CancellationSignal::new();
    let mut wait = CancellationWait::new(&signal.0);
    signal.0.cancelled.store(true, Ordering::Release);

    assert!(wait.poll_after_precheck(Waker::noop().clone()).is_ready());
    assert!(wait.completed);
    assert_eq!(wait.token, None);
    assert!(
        signal
            .0
            .registry
            .lock()
            .expect("registry")
            .waiters
            .is_empty()
    );
}

#[test]
fn cancellation_between_registration_and_drain_has_no_lost_wake() {
    let signal = CancellationSignal::new();
    let mut wait = llm_gateway::CancellationSignal::cancelled(&signal);
    let count = Arc::new(WakeCount(AtomicUsize::new(0)));
    let waker = Waker::from(Arc::clone(&count));
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );

    let registry = signal.0.registry.lock().expect("registry");
    let cancel_signal = signal.clone();
    let (started_tx, started_rx) = std::sync::mpsc::channel();
    let cancel = std::thread::spawn(move || {
        started_tx.send(()).expect("start notification");
        cancel_signal.cancel();
    });
    started_rx.recv().expect("cancel started");
    while !signal.is_cancelled() {
        std::thread::yield_now();
    }
    assert_eq!(registry.waiters.len(), 1);
    drop(registry);
    cancel.join().expect("cancel thread");

    assert_eq!(count.0.load(AtomicOrdering::Relaxed), 1);
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_ready()
    );
}

struct RegistryInspectWake {
    state: Arc<CancellationState>,
    wake_count: AtomicUsize,
}
impl Wake for RegistryInspectWake {
    fn wake(self: Arc<Self>) {
        assert!(
            self.state
                .registry
                .lock()
                .expect("registry")
                .waiters
                .is_empty()
        );
        self.wake_count.fetch_add(1, AtomicOrdering::Relaxed);
    }
}

#[test]
fn cancellation_drains_registry_before_waking() {
    let signal = CancellationSignal::new();
    let inspect = Arc::new(RegistryInspectWake {
        state: Arc::clone(&signal.0),
        wake_count: AtomicUsize::new(0),
    });
    let waker = Waker::from(Arc::clone(&inspect));
    let mut wait = llm_gateway::CancellationSignal::cancelled(&signal);
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(&waker))
            .is_pending()
    );

    let worker_signal = signal.clone();
    let (done_tx, done_rx) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        worker_signal.cancel();
        done_tx.send(()).expect("completion notification");
    });
    done_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("cancellation must not wake while holding the registry lock");
    worker.join().expect("cancel worker");
    assert_eq!(inspect.wake_count.load(AtomicOrdering::Relaxed), 1);
}

#[test]
fn cloned_cancellation_signals_broadcast_to_all_waiters() {
    let signal = CancellationSignal::new();
    let clone = signal.clone();
    let mut original_wait = llm_gateway::CancellationSignal::cancelled(&signal);
    let mut clone_wait = llm_gateway::CancellationSignal::cancelled(&clone);
    let original_count = Arc::new(WakeCount(AtomicUsize::new(0)));
    let clone_count = Arc::new(WakeCount(AtomicUsize::new(0)));
    let original_waker = Waker::from(Arc::clone(&original_count));
    let clone_waker = Waker::from(Arc::clone(&clone_count));
    assert!(
        original_wait
            .as_mut()
            .poll(&mut Context::from_waker(&original_waker))
            .is_pending()
    );
    assert!(
        clone_wait
            .as_mut()
            .poll(&mut Context::from_waker(&clone_waker))
            .is_pending()
    );

    clone.cancel();
    assert_eq!(original_count.0.load(AtomicOrdering::Relaxed), 1);
    assert_eq!(clone_count.0.load(AtomicOrdering::Relaxed), 1);
    assert!(
        original_wait
            .as_mut()
            .poll(&mut Context::from_waker(&original_waker))
            .is_ready()
    );
    assert!(
        clone_wait
            .as_mut()
            .poll(&mut Context::from_waker(&clone_waker))
            .is_ready()
    );
}

#[test]
fn missing_registration_fails_closed_without_leaking() {
    let signal = CancellationSignal::new();
    let mut wait = llm_gateway::CancellationSignal::cancelled(&signal);
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_pending()
    );
    signal.0.registry.lock().expect("registry").waiters.clear();

    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );
    assert!(
        wait.as_mut()
            .poll(&mut Context::from_waker(Waker::noop()))
            .is_ready()
    );
    assert!(
        signal
            .0
            .registry
            .lock()
            .expect("registry")
            .waiters
            .is_empty()
    );
}

fn poison_registry(state: Arc<CancellationState>) {
    let _ = std::thread::spawn(move || {
        let _guard = state.registry.lock().expect("registry");
        panic!("poison registry");
    })
    .join();
}

#[test]
fn poisoned_registry_poll_removes_only_own_waiter_and_cancel_wakes_survivor() {
    let signal = CancellationSignal::new();
    let mut failed_closed = llm_gateway::CancellationSignal::cancelled(&signal);
    let mut survivor = llm_gateway::CancellationSignal::cancelled(&signal);
    let failed_count = Arc::new(WakeCount(AtomicUsize::new(0)));
    let survivor_count = Arc::new(WakeCount(AtomicUsize::new(0)));
    let failed_waker = Waker::from(Arc::clone(&failed_count));
    let survivor_waker = Waker::from(Arc::clone(&survivor_count));
    assert!(
        failed_closed
            .as_mut()
            .poll(&mut Context::from_waker(&failed_waker))
            .is_pending()
    );
    assert!(
        survivor
            .as_mut()
            .poll(&mut Context::from_waker(&survivor_waker))
            .is_pending()
    );

    poison_registry(Arc::clone(&signal.0));
    assert!(
        failed_closed
            .as_mut()
            .poll(&mut Context::from_waker(&failed_waker))
            .is_ready()
    );
    {
        let registry = signal
            .0
            .registry
            .lock()
            .expect_err("registry remains poisoned")
            .into_inner();
        assert_eq!(registry.waiters.len(), 1);
        assert!(registry.waiters.contains_key(&1));
    }
    assert_eq!(failed_count.0.load(AtomicOrdering::Relaxed), 0);
    assert_eq!(survivor_count.0.load(AtomicOrdering::Relaxed), 0);

    signal.cancel();
    assert!(signal.is_cancelled());
    assert_eq!(failed_count.0.load(AtomicOrdering::Relaxed), 0);
    assert_eq!(survivor_count.0.load(AtomicOrdering::Relaxed), 1);
    assert!(
        survivor
            .as_mut()
            .poll(&mut Context::from_waker(&survivor_waker))
            .is_ready()
    );
}

#[test]
fn dropping_wait_against_poisoned_registry_removes_only_it_and_survivor_wakes() {
    let signal = CancellationSignal::new();
    let mut removed = llm_gateway::CancellationSignal::cancelled(&signal);
    let mut survivor = llm_gateway::CancellationSignal::cancelled(&signal);
    let removed_count = Arc::new(WakeCount(AtomicUsize::new(0)));
    let survivor_count = Arc::new(WakeCount(AtomicUsize::new(0)));
    let removed_waker = Waker::from(Arc::clone(&removed_count));
    let survivor_waker = Waker::from(Arc::clone(&survivor_count));
    assert!(
        removed
            .as_mut()
            .poll(&mut Context::from_waker(&removed_waker))
            .is_pending()
    );
    assert!(
        survivor
            .as_mut()
            .poll(&mut Context::from_waker(&survivor_waker))
            .is_pending()
    );

    poison_registry(Arc::clone(&signal.0));
    drop(removed);
    {
        let registry = signal
            .0
            .registry
            .lock()
            .expect_err("registry remains poisoned")
            .into_inner();
        assert_eq!(registry.waiters.len(), 1);
        assert!(registry.waiters.contains_key(&1));
    }
    assert_eq!(removed_count.0.load(AtomicOrdering::Relaxed), 0);
    assert_eq!(survivor_count.0.load(AtomicOrdering::Relaxed), 0);

    signal.cancel();
    assert_eq!(removed_count.0.load(AtomicOrdering::Relaxed), 0);
    assert_eq!(survivor_count.0.load(AtomicOrdering::Relaxed), 1);
    assert!(
        survivor
            .as_mut()
            .poll(&mut Context::from_waker(&survivor_waker))
            .is_ready()
    );
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
