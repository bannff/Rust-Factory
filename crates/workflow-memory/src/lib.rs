#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]

//! Deterministic in-memory workflow adapters for local use and tests.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use agent::AgentId;
use workflow::{
    AgentInvocationRequest, AgentInvocationResult, AgentInvoker, CreateRun, InvocationEvidence,
    InvocationEvidenceSink, LogicalId, StartIdentity, StartKey, Transition, TransitionResult,
    WorkflowDefinitionCatalog, WorkflowDefinitionV1, WorkflowError, WorkflowStore,
    transition_is_valid,
};

#[derive(Clone, Default)]
pub struct InMemoryWorkflowStore {
    state: Arc<Mutex<State>>,
}
#[derive(Default)]
struct State {
    runs: BTreeMap<LogicalId, workflow::Run>,
    identities: BTreeMap<StartIdentity, LogicalId>,
    keys: BTreeMap<StartKey, StartIdentity>,
}
impl WorkflowStore for InMemoryWorkflowStore {
    fn create_or_return(
        &self,
        identity: StartIdentity,
        run: workflow::Run,
    ) -> Result<CreateRun, WorkflowError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| WorkflowError::AdapterFailure)?;
        if let Some(id) = state.identities.get(&identity) {
            return state
                .runs
                .get(id)
                .cloned()
                .map(CreateRun::Existing)
                .ok_or(WorkflowError::AdapterFailure);
        }
        if state.keys.contains_key(&identity.key) {
            return Ok(CreateRun::Conflict);
        }
        if run.context.tenant_id != identity.key.tenant_id
            || run.workflow_id != identity.key.workflow_id
            || run.workflow_version != identity.key.workflow_version
            || run.run_key != identity.key.run_key
            || run.input_digest != identity.input_digest
            || run.status != workflow::RunStatus::Pending
            || run.revision != 0
            || run.terminal_reason.is_some()
            || run.attempt.is_some()
            || !run.events.is_empty()
            || run.max_evidence_bytes == 0
            || run.max_evidence_bytes > workflow::MAX_EVIDENCE_BYTES
        {
            return Err(WorkflowError::InvalidRequest);
        }
        state.keys.insert(identity.key.clone(), identity.clone());
        state.identities.insert(identity, run.id.clone());
        state.runs.insert(run.id.clone(), run.clone());
        Ok(CreateRun::Created(run))
    }
    fn get(
        &self,
        tenant: &LogicalId,
        run: &LogicalId,
    ) -> Result<Option<workflow::Run>, WorkflowError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| WorkflowError::AdapterFailure)?
            .runs
            .get(run)
            .filter(|value| &value.context.tenant_id == tenant)
            .cloned())
    }
    fn list(&self, tenant: &LogicalId) -> Result<Vec<workflow::Run>, WorkflowError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| WorkflowError::AdapterFailure)?
            .runs
            .values()
            .filter(|value| &value.context.tenant_id == tenant)
            .cloned()
            .collect())
    }
    fn transition(
        &self,
        tenant: &LogicalId,
        run_id: &LogicalId,
        revision: u64,
        status: workflow::RunStatus,
        transition: Transition,
    ) -> Result<TransitionResult, WorkflowError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| WorkflowError::AdapterFailure)?;
        let Some(current) = state.runs.get(run_id) else {
            return Ok(TransitionResult::NotFound);
        };
        if &current.context.tenant_id != tenant {
            return Ok(TransitionResult::NotFound);
        }
        if current.revision != revision || !transition_is_valid(current, status, &transition) {
            return Ok(TransitionResult::Conflict);
        }
        let run = state
            .runs
            .get_mut(run_id)
            .ok_or(WorkflowError::AdapterFailure)?;
        run.status = transition.status;
        run.terminal_reason = transition.terminal_reason;
        run.attempt = transition.attempt;
        run.events.extend(transition.events);
        run.revision += 1;
        Ok(TransitionResult::Applied(run.clone()))
    }
}

#[derive(Clone, Default)]
pub struct StaticWorkflowCatalog {
    definitions: BTreeMap<(LogicalId, workflow::WorkflowVersion), WorkflowDefinitionV1>,
}
impl StaticWorkflowCatalog {
    #[must_use]
    pub fn new(definitions: impl IntoIterator<Item = WorkflowDefinitionV1>) -> Self {
        Self {
            definitions: definitions
                .into_iter()
                .map(|item| ((item.id.clone(), item.version), item))
                .collect(),
        }
    }
}
impl WorkflowDefinitionCatalog for StaticWorkflowCatalog {
    fn resolve(
        &self,
        id: &LogicalId,
        version: workflow::WorkflowVersion,
    ) -> Result<Option<WorkflowDefinitionV1>, WorkflowError> {
        Ok(self.definitions.get(&(id.clone(), version)).cloned())
    }
}

#[derive(Clone, Debug)]
pub enum StaticInvocation {
    Succeed {
        capability_scope_digest: String,
        evidence: Vec<InvocationEvidence>,
    },
    Fail,
}
#[derive(Clone, Debug)]
pub struct StaticAgentInvoker {
    agents: Vec<AgentId>,
    invocation: StaticInvocation,
}
impl StaticAgentInvoker {
    #[must_use]
    pub fn new(agents: Vec<AgentId>, invocation: StaticInvocation) -> Self {
        Self { agents, invocation }
    }
}
impl AgentInvoker for StaticAgentInvoker {
    fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError> {
        Ok(self.agents.contains(id))
    }
    fn invoke(
        &self,
        request: AgentInvocationRequest,
        evidence: &mut dyn InvocationEvidenceSink,
    ) -> Result<AgentInvocationResult, WorkflowError> {
        if request.cancellation.is_cancelled() || std::time::Instant::now() >= request.deadline {
            return Err(WorkflowError::Conflict);
        }
        match &self.invocation {
            StaticInvocation::Succeed {
                capability_scope_digest,
                evidence: items,
            } => {
                for item in items {
                    evidence.emit(item.clone())?;
                }
                Ok(AgentInvocationResult {
                    capability_scope_digest: capability_scope_digest.clone(),
                })
            }
            StaticInvocation::Fail => Err(WorkflowError::AdapterFailure),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use workflow::{
        AgentStep, Attempt, AttemptStatus, RequestContext, RunStatus, TerminalReason,
        WorkflowBudget, WorkflowEvent, WorkflowRunner, WorkflowVersion,
    };

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
    fn definition() -> WorkflowDefinitionV1 {
        WorkflowDefinitionV1 {
            id: id("workflow"),
            version: WorkflowVersion::V1,
            step: AgentStep {
                agent_id: AgentId::new("agent").expect("agent"),
            },
            budget: WorkflowBudget::default(),
        }
    }
    fn success() -> StaticInvocation {
        StaticInvocation::Succeed {
            capability_scope_digest: "scope".to_owned(),
            evidence: vec![
                InvocationEvidence::new("agent_event", "evidence").expect("evidence"),
                InvocationEvidence::new("result", "result").expect("result"),
            ],
        }
    }
    fn runner(
        store: InMemoryWorkflowStore,
    ) -> WorkflowRunner<InMemoryWorkflowStore, StaticWorkflowCatalog, StaticAgentInvoker> {
        WorkflowRunner::new(
            store,
            StaticWorkflowCatalog::new([definition()]),
            StaticAgentInvoker::new(vec![AgentId::new("agent").expect("agent")], success()),
        )
    }

    #[test]
    fn duplicate_start_replays_and_conflicting_key_is_rejected() {
        let service = runner(InMemoryWorkflowStore::default());
        let first = service
            .start(
                context("tenant"),
                id("workflow"),
                WorkflowVersion::V1,
                "key".to_owned(),
                r#"{"b":2,"a":1}"#.to_owned(),
            )
            .expect("start");
        let duplicate = service
            .start(
                context("tenant"),
                id("workflow"),
                WorkflowVersion::V1,
                "key".to_owned(),
                r#"{"a":1,"b":2}"#.to_owned(),
            )
            .expect("replay");
        assert_eq!(first, duplicate);
        assert_eq!(first.status, RunStatus::Succeeded);
        assert_eq!(
            service.start(
                context("tenant"),
                id("workflow"),
                WorkflowVersion::V1,
                "key".to_owned(),
                r#"{"a":3}"#.to_owned()
            ),
            Err(WorkflowError::RunKeyConflict)
        );
    }
    #[test]
    fn typed_start_keys_do_not_confuse_prefixes() {
        let store = InMemoryWorkflowStore::default();
        let first = StartIdentity {
            key: StartKey {
                tenant_id: id("tenant"),
                workflow_id: id("workflow"),
                workflow_version: WorkflowVersion::V1,
                run_key: "same".to_owned(),
            },
            input_digest: "digest-a".to_owned(),
        };
        let second = StartIdentity {
            key: StartKey {
                tenant_id: id("tenant"),
                workflow_id: id("workflow"),
                workflow_version: WorkflowVersion::V1,
                run_key: "same:other".to_owned(),
            },
            input_digest: "digest".to_owned(),
        };
        let run = |identity: &StartIdentity, name: &str| workflow::Run {
            id: id(name),
            context: context("tenant"),
            workflow_id: id("workflow"),
            workflow_version: WorkflowVersion::V1,
            run_key: identity.key.run_key.clone(),
            input_digest: identity.input_digest.clone(),
            max_evidence_bytes: workflow::MAX_EVIDENCE_BYTES,
            status: RunStatus::Pending,
            revision: 0,
            terminal_reason: None,
            attempt: None,
            events: vec![],
        };
        assert!(matches!(
            store
                .create_or_return(first.clone(), run(&first, "runa"))
                .expect("first"),
            CreateRun::Created(_)
        ));
        assert!(matches!(
            store
                .create_or_return(second.clone(), run(&second, "runb"))
                .expect("second"),
            CreateRun::Created(_)
        ));
    }
    #[test]
    fn tenant_scoping_hides_runs_from_other_tenants() {
        let store = InMemoryWorkflowStore::default();
        let service = runner(store.clone());
        let summary = service
            .start(
                context("tenant-a"),
                id("workflow"),
                WorkflowVersion::V1,
                "key".to_owned(),
                "{}".to_owned(),
            )
            .expect("start");

        assert_eq!(
            service.get(&id("tenant-b"), summary.id.clone()),
            Err(WorkflowError::NotFound),
            "a tenant must not discover another tenant's run by ID"
        );
        assert!(
            service.list(&id("tenant-b")).expect("list").is_empty(),
            "a tenant must not receive another tenant's runs in a listing"
        );
    }

    #[test]
    fn invalid_transition_is_rejected_without_changes() {
        let store = InMemoryWorkflowStore::default();
        let service = runner(store.clone());
        let started = service
            .start(
                context("tenant"),
                id("workflow"),
                WorkflowVersion::V1,
                "key".to_owned(),
                "{}".to_owned(),
            )
            .expect("start");
        let before = store
            .get(&id("tenant"), &started.id)
            .expect("get")
            .expect("run");
        let invalid = Transition {
            status: RunStatus::Failed,
            terminal_reason: None,
            attempt: before.attempt.clone(),
            events: vec![WorkflowEvent {
                sequence: before.events.len() as u64 + 1,
                kind: "bad".to_owned(),
                data: String::new(),
            }],
        };
        assert_eq!(
            store
                .transition(
                    &id("tenant"),
                    &started.id,
                    before.revision,
                    before.status,
                    invalid
                )
                .expect("transition"),
            TransitionResult::Conflict
        );
        assert_eq!(
            store
                .get(&id("tenant"), &started.id)
                .expect("get")
                .expect("run"),
            before
        );
    }
    #[test]
    fn oversized_evidence_fails_run_without_stranding_it() {
        let store = InMemoryWorkflowStore::default();
        let mut evidence = vec![
            InvocationEvidence::new("agent_event", "x").expect("evidence");
            workflow::MAX_EVENTS - 1
        ];
        evidence.push(InvocationEvidence::new("result", "x").expect("result"));
        let service = WorkflowRunner::new(
            store.clone(),
            StaticWorkflowCatalog::new([definition()]),
            StaticAgentInvoker::new(
                vec![AgentId::new("agent").expect("agent")],
                StaticInvocation::Succeed {
                    capability_scope_digest: "scope".to_owned(),
                    evidence,
                },
            ),
        );
        assert_eq!(
            service.start(
                context("tenant"),
                id("workflow"),
                WorkflowVersion::V1,
                "key".to_owned(),
                "{}".to_owned()
            ),
            Err(WorkflowError::LimitExceeded)
        );
        let run = store.list(&id("tenant")).expect("list").pop().expect("run");
        assert_eq!(run.status, RunStatus::Failed);
        assert_eq!(run.terminal_reason, Some(TerminalReason::InvocationFailed));
        assert_eq!(
            service.cancel(&id("tenant"), run.id.clone()),
            Ok(workflow::RunSummary::from(&run))
        );
    }
    #[test]
    fn transition_requires_continuous_attempt_and_ordered_events() {
        let previous = Attempt {
            id: id("attempt"),
            agent_id: AgentId::new("agent").expect("agent"),
            effective_capability_ceiling: agent::EffectiveCapabilityCeilingV1 {
                allowed_tool_ids: vec![],
                memory_enabled: false,
                knowledge_enabled: false,
                sandbox_execution_allowed: false,
                communication_allowed: false,
            },
            policy_decision_digest: "0".repeat(64),
            capability_scope_digest: None,
            status: AttemptStatus::Running,
            result: None,
            error: None,
        };
        let run = workflow::Run {
            id: id("run"),
            context: context("tenant"),
            workflow_id: id("workflow"),
            workflow_version: WorkflowVersion::V1,
            run_key: "key".to_owned(),
            input_digest: "digest".to_owned(),
            max_evidence_bytes: workflow::MAX_EVIDENCE_BYTES,
            status: RunStatus::Running,
            revision: 1,
            terminal_reason: None,
            attempt: Some(previous.clone()),
            events: vec![WorkflowEvent {
                sequence: 1,
                kind: "started".to_owned(),
                data: String::new(),
            }],
        };
        let mut changed = previous;
        changed.id = id("other");
        let transition = Transition {
            status: RunStatus::Failed,
            terminal_reason: Some(TerminalReason::InvocationFailed),
            attempt: Some(changed),
            events: vec![WorkflowEvent {
                sequence: 3,
                kind: "failed".to_owned(),
                data: String::new(),
            }],
        };
        assert!(!transition_is_valid(&run, RunStatus::Running, &transition));
    }
}

#[cfg(test)]
mod cancellation_tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;
    use workflow::{
        AgentStep, RequestContext, RunStatus, WorkflowBudget, WorkflowRunner, WorkflowVersion,
    };

    #[derive(Clone)]
    struct BlockingInvoker {
        entered: Arc<Barrier>,
        release: Arc<Barrier>,
        agent: AgentId,
    }
    impl AgentInvoker for BlockingInvoker {
        fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError> {
            Ok(id == &self.agent)
        }
        fn invoke(
            &self,
            request: AgentInvocationRequest,
            evidence: &mut dyn InvocationEvidenceSink,
        ) -> Result<AgentInvocationResult, WorkflowError> {
            self.entered.wait();
            self.release.wait();
            if request.cancellation.is_cancelled() {
                return Err(WorkflowError::Conflict);
            }
            evidence.emit(InvocationEvidence::new("result", "late-result").expect("evidence"))?;
            Ok(AgentInvocationResult {
                capability_scope_digest: "scope".to_owned(),
            })
        }
    }
    fn id(value: &str) -> LogicalId {
        LogicalId::new(value).expect("id")
    }
    fn context() -> RequestContext {
        RequestContext {
            tenant_id: id("tenant"),
            principal_id: id("principal"),
            request_id: id("request"),
            correlation_id: id("correlation"),
        }
    }
    #[test]
    fn cancellation_wins_over_a_late_completion_and_requires_local_registration() {
        let store = InMemoryWorkflowStore::default();
        let agent = AgentId::new("agent").expect("agent");
        let entered = Arc::new(Barrier::new(2));
        let release = Arc::new(Barrier::new(2));
        let runner = Arc::new(WorkflowRunner::new(
            store.clone(),
            StaticWorkflowCatalog::new([WorkflowDefinitionV1 {
                id: id("workflow"),
                version: WorkflowVersion::V1,
                step: AgentStep {
                    agent_id: agent.clone(),
                },
                budget: WorkflowBudget::default(),
            }]),
            BlockingInvoker {
                entered: entered.clone(),
                release: release.clone(),
                agent,
            },
        ));
        let worker = runner.clone();
        let handle = thread::spawn(move || {
            worker.start(
                context(),
                id("workflow"),
                WorkflowVersion::V1,
                "key".to_owned(),
                "{}".to_owned(),
            )
        });
        entered.wait();
        let running = store.list(&id("tenant")).expect("list").pop().expect("run");
        let cancelled = runner.cancel(&id("tenant"), running.id).expect("cancel");
        assert_eq!(cancelled.status, RunStatus::Cancelled);
        release.wait();
        assert_eq!(
            handle.join().expect("thread").expect("result").status,
            RunStatus::Cancelled
        );
    }
}

#[cfg(test)]
mod policy_attempt_tests {
    use super::*;
    use agent::EffectiveCapabilityCeilingV1;
    use workflow::{
        AgentStep, Attempt, AttemptStatus, InvocationPolicy, RequestContext, Run, RunStatus,
        TerminalReason, WorkflowBudget, WorkflowEvent, WorkflowRunner, WorkflowVersion,
        transition_is_valid,
    };

    fn id(value: &str) -> LogicalId {
        LogicalId::new(value).expect("logical id")
    }

    fn context() -> RequestContext {
        RequestContext {
            tenant_id: id("tenant"),
            principal_id: id("principal"),
            request_id: id("request"),
            correlation_id: id("correlation"),
        }
    }

    fn ceiling() -> EffectiveCapabilityCeilingV1 {
        EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: vec!["tool".to_owned()],
            memory_enabled: true,
            knowledge_enabled: false,
            sandbox_execution_allowed: false,
            communication_allowed: false,
        }
    }

    fn running_run() -> Run {
        Run {
            id: id("run"),
            context: context(),
            workflow_id: id("workflow"),
            workflow_version: WorkflowVersion::V1,
            run_key: "key".to_owned(),
            input_digest: "digest".to_owned(),
            max_evidence_bytes: workflow::MAX_EVIDENCE_BYTES,
            status: RunStatus::Running,
            revision: 1,
            terminal_reason: None,
            attempt: Some(Attempt {
                id: id("attempt"),
                agent_id: AgentId::new("agent").expect("agent"),
                effective_capability_ceiling: ceiling(),
                policy_decision_digest: "a".repeat(64),
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
        }
    }

    #[test]
    fn start_persists_the_validated_policy_evidence_on_the_attempt() {
        let store = InMemoryWorkflowStore::default();
        let runner = WorkflowRunner::new(
            store.clone(),
            StaticWorkflowCatalog::new([WorkflowDefinitionV1 {
                id: id("workflow"),
                version: WorkflowVersion::V1,
                step: AgentStep {
                    agent_id: AgentId::new("agent").expect("agent"),
                },
                budget: WorkflowBudget::default(),
            }]),
            StaticAgentInvoker::new(
                vec![AgentId::new("agent").expect("agent")],
                StaticInvocation::Succeed {
                    capability_scope_digest: "scope".to_owned(),
                    evidence: vec![InvocationEvidence::new("result", "output").expect("result")],
                },
            ),
        );
        let policy = InvocationPolicy {
            effective_capability_ceiling: ceiling(),
            policy_decision_digest: "a".repeat(64),
        };

        let summary = runner
            .start_with_policy(
                context(),
                id("workflow"),
                WorkflowVersion::V1,
                "key".to_owned(),
                "{}".to_owned(),
                policy.clone(),
            )
            .expect("start");
        let attempt = store
            .get(&id("tenant"), &summary.id)
            .expect("get")
            .expect("run")
            .attempt
            .expect("attempt");
        assert_eq!(
            attempt.effective_capability_ceiling,
            policy.effective_capability_ceiling
        );
        assert_eq!(
            attempt.policy_decision_digest,
            policy.policy_decision_digest
        );
        assert_eq!(attempt.capability_scope_digest.as_deref(), Some("scope"));
    }

    #[test]
    fn terminal_transitions_reject_ceiling_or_decision_digest_mutation() {
        let run = running_run();
        for (status, reason, result, error) in [
            (
                RunStatus::Succeeded,
                TerminalReason::Completed,
                Some("output".to_owned()),
                None,
            ),
            (
                RunStatus::Failed,
                TerminalReason::InvocationFailed,
                None,
                Some("failed".to_owned()),
            ),
            (RunStatus::Cancelled, TerminalReason::Cancelled, None, None),
        ] {
            let mut attempt = run.attempt.clone().expect("attempt");
            attempt.status = match status {
                RunStatus::Succeeded => AttemptStatus::Succeeded,
                RunStatus::Failed => AttemptStatus::Failed,
                RunStatus::Cancelled => AttemptStatus::Cancelled,
                RunStatus::Pending | RunStatus::Running => unreachable!(),
            };
            attempt.result = result;
            attempt.error = error;
            let transition = Transition {
                status,
                terminal_reason: Some(reason),
                attempt: Some(attempt.clone()),
                events: vec![WorkflowEvent {
                    sequence: 2,
                    kind: "terminal".to_owned(),
                    data: String::new(),
                }],
            };
            assert!(transition_is_valid(&run, RunStatus::Running, &transition));

            let mut changed_ceiling = transition.clone();
            changed_ceiling
                .attempt
                .as_mut()
                .expect("attempt")
                .effective_capability_ceiling
                .memory_enabled = false;
            assert!(!transition_is_valid(
                &run,
                RunStatus::Running,
                &changed_ceiling
            ));

            let mut changed_digest = transition;
            changed_digest
                .attempt
                .as_mut()
                .expect("attempt")
                .policy_decision_digest = "b".repeat(64);
            assert!(!transition_is_valid(
                &run,
                RunStatus::Running,
                &changed_digest
            ));
        }
    }
}
