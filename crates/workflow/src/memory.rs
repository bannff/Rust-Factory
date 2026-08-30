//! Deterministic in-memory workflow adapters for local use and tests.
//!
//! Deterministic process-local adapters, enabled by the `memory` feature.
//! No persistence, recovery, lease, or cross-process cancellation guarantee.

#![allow(clippy::missing_errors_doc)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::{
    AgentInvocationRequest, AgentInvocationResult, AgentInvoker, CreateRun, InvocationEvidence,
    InvocationEvidenceSink, LogicalId, StartIdentity, StartKey, Transition, TransitionResult,
    WorkflowDefinitionCatalog, WorkflowDefinitionV1, WorkflowError, WorkflowStore,
    transition_is_valid,
};
use agent::AgentId;

#[derive(Clone, Default)]
pub struct InMemoryWorkflowStore {
    state: Arc<Mutex<State>>,
}
#[derive(Default)]
struct State {
    runs: BTreeMap<LogicalId, crate::Run>,
    identities: BTreeMap<StartIdentity, LogicalId>,
    keys: BTreeMap<StartKey, StartIdentity>,
}
impl WorkflowStore for InMemoryWorkflowStore {
    fn create_or_return(
        &self,
        identity: StartIdentity,
        run: crate::Run,
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
            || run.status != crate::RunStatus::Pending
            || run.revision != 0
            || run.terminal_reason.is_some()
            || run.attempt.is_some()
            || !run.events.is_empty()
            || run.max_evidence_bytes == 0
            || run.max_evidence_bytes > crate::MAX_EVIDENCE_BYTES
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
    ) -> Result<Option<crate::Run>, WorkflowError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| WorkflowError::AdapterFailure)?
            .runs
            .get(run)
            .filter(|value| &value.context.tenant_id == tenant)
            .cloned())
    }
    fn list(&self, tenant: &LogicalId) -> Result<Vec<crate::Run>, WorkflowError> {
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
        status: crate::RunStatus,
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
    definitions: BTreeMap<(LogicalId, crate::WorkflowVersion), WorkflowDefinitionV1>,
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
        version: crate::WorkflowVersion,
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
    fn invoke<'a>(
        &'a self,
        _request: AgentInvocationRequest,
        control: llm_gateway::InvocationControl<'a>,
        evidence: &'a mut dyn InvocationEvidenceSink,
    ) -> crate::AgentInvocationFuture<'a> {
        Box::pin(async move {
            control.preflight().map_err(|error| match error {
                llm_gateway::LlmError::Cancelled => WorkflowError::Cancelled,
                llm_gateway::LlmError::DeadlineExceeded => WorkflowError::DeadlineExceeded,
                _ => WorkflowError::AdapterFailure,
            })?;
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
        })
    }
}

#[cfg(test)]
#[path = "memory_tests.rs"]
mod tests;
