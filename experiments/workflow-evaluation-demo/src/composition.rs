//! Static Agent -> Workflow -> Evaluation -> Observability composition.
//!
//! Built once at process startup. Tenant, principal, and workflow identity are
//! fixed constants here; they are never accepted from a caller or tool input.

use std::collections::BTreeMap;
use std::error::Error;

use crate::adapters::{
    DenyScheduledSandbox, FixedClock, ProjectDeadlineFactory, RuntimeInvoker, StrictEvidenceReader,
};
use evaluation::{
    CreateOrMatch, CriterionV1, EvaluationDefinitionV1, EvaluationResultV1, EvaluationService,
    Verdict,
};
use observability::{
    EventName, EventTarget, Severity, TelemetryContext, TelemetryEventV1, TelemetryQueryV1,
    TelemetryRecordV1, TelemetryService,
};
use workflow::memory::{InMemoryWorkflowStore, StaticWorkflowCatalog};
use workflow::{
    AgentStep, LogicalId, RequestContext, RunSummary, WorkflowBudget, WorkflowDefinitionV1,
    WorkflowRunner, WorkflowStore, WorkflowVersion,
};

pub const TENANT: &str = "demo-tenant";
pub const PRINCIPAL: &str = "demo-host";
pub const REQUEST: &str = "demo-request";
pub const CORRELATION: &str = "demo-correlation";
pub const AGENT: &str = "demo-agent";
pub const WORKFLOW: &str = "demo-workflow";
const RUN_KEY: &str = "demo-run";
/// Bound on caller-supplied lookup/query identifiers and limits. Kept small and
/// explicit rather than reusing an unrelated crate's internal limit constant.
pub const MAX_RUN_ID_BYTES: usize = 256;
pub const MAX_TELEMETRY_QUERY_LIMIT: usize = 50;
/// Local telemetry retention capacity per tenant. Kept equal to
/// [`MAX_TELEMETRY_QUERY_LIMIT`] so a fully-bounded query can actually be
/// satisfied; both the documented and enforced query range are `1..=50`.
const TELEMETRY_CAPACITY: usize = MAX_TELEMETRY_QUERY_LIMIT;

type Invoker = RuntimeInvoker<
    llm_gateway::r#static::StaticProvider,
    agent::FixedToolRegistry,
    agent::InMemoryMemoryStore,
    knowledge::r#static::StaticKnowledgeIndex,
    DenyScheduledSandbox,
>;
type Runner = WorkflowRunner<InMemoryWorkflowStore, StaticWorkflowCatalog, Invoker>;
type Evaluation = EvaluationService<
    StrictEvidenceReader<InMemoryWorkflowStore>,
    evaluation::memory::InMemoryEvaluationStore,
    evaluation::local::DeterministicCriteriaEvaluator,
>;
type Telemetry = TelemetryService<
    observability::local::LocalTelemetry,
    observability::local::LocalTelemetry,
    FixedClock,
>;

/// A bounded outcome describing one deterministic run of the demo pipeline.
#[derive(Clone, Debug)]
pub struct RunDemoOutcome {
    pub run_id: String,
    pub evidence_digest: String,
    pub result_digest: String,
    pub verdict: &'static str,
}

/// A tenant-fixed, bounded projection of one stored workflow run.
#[derive(Clone, Debug)]
pub struct RunView {
    pub run_id: String,
    pub status: &'static str,
    pub terminal_reason: Option<&'static str>,
    pub output: Option<String>,
}

/// One telemetry record projected to its allowlisted attributes only.
#[derive(Clone, Debug, Default)]
pub struct TelemetryAttributes {
    pub workflow_run_id: Option<String>,
    pub evidence_digest: Option<String>,
    pub result_digest: Option<String>,
    pub verdict: Option<String>,
}

/// Bounded, non-leaking errors surfaced by composition operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CompositionError {
    InvalidRequest,
    LimitExceeded,
    OperationFailed,
}

/// Owns the one static demo composition: a deterministic agent runtime, an
/// in-memory workflow runner, a deterministic evaluation service, and
/// process-local telemetry. Constructed once at startup and shared by every
/// MCP tool call. No caller input ever reaches identity or construction.
pub struct Composition {
    runner: Runner,
    workflow_store: InMemoryWorkflowStore,
    evaluation: Evaluation,
    evaluation_definition: EvaluationDefinitionV1,
    telemetry: Telemetry,
    local_telemetry: observability::local::LocalTelemetry,
    tenant: LogicalId,
    request_context: RequestContext,
    workflow_id: LogicalId,
}

impl Composition {
    pub fn new() -> Result<Self, Box<dyn Error>> {
        let agent_id = agent::AgentId::new(AGENT)?;
        let registry = agent::AgentRegistry::new(
            vec![agent_definition(agent_id.clone())],
            agent::InMemoryDefinitionStore::default(),
            agent::StaticReferenceCatalog::new(["static.demo".to_owned()], [], [], []),
        )?;
        let fixture = llm_gateway::r#static::StaticFixture::new(
            "done",
            vec![],
            None,
            llm_gateway::FinishReason::Stop,
            None,
            llm_gateway::IdempotencyDisposition::Accepted,
        )?;
        let invoker = RuntimeInvoker {
            registry,
            provider: llm_gateway::r#static::StaticProvider::success(fixture),
            tools: agent::FixedToolRegistry::default(),
            memory: agent::InMemoryMemoryStore::default(),
            knowledge: knowledge::r#static::StaticKnowledgeIndex::new(vec![])?,
            sandbox: DenyScheduledSandbox,
        };

        let workflow_id = LogicalId::new(WORKFLOW)?;
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
        let workflow_store = InMemoryWorkflowStore::default();
        let runner = WorkflowRunner::new(
            workflow_store.clone(),
            StaticWorkflowCatalog::new([definition]),
            invoker,
            Box::new(ProjectDeadlineFactory),
            Box::new(llm_gateway::tokio_cancellation::TokioCancellationSignalFactory),
        );

        let evaluation = EvaluationService::new(
            StrictEvidenceReader(workflow_store.clone()),
            evaluation::memory::InMemoryEvaluationStore::new(),
            evaluation::local::DeterministicCriteriaEvaluator,
        );
        let evaluation_definition = EvaluationDefinitionV1 {
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
        };

        let local_telemetry = observability::local::LocalTelemetry::new(TELEMETRY_CAPACITY)?;
        let telemetry = TelemetryService::new(
            local_telemetry.clone(),
            local_telemetry.clone(),
            FixedClock,
            MAX_TELEMETRY_QUERY_LIMIT,
        )?;

        let request_context = RequestContext {
            tenant_id: LogicalId::new(TENANT)?,
            principal_id: LogicalId::new(PRINCIPAL)?,
            request_id: LogicalId::new(REQUEST)?,
            correlation_id: LogicalId::new(CORRELATION)?,
        };

        Ok(Self {
            runner,
            workflow_store,
            evaluation,
            evaluation_definition,
            telemetry,
            local_telemetry,
            tenant: LogicalId::new(TENANT)?,
            request_context,
            workflow_id,
        })
    }

    /// Runs one Agent -> Workflow -> Evaluation -> Observability cycle using the
    /// fixed demo identity and a deny-all capability ceiling. Idempotent: a
    /// repeated call replays the same stored run and evaluation result.
    pub async fn run_demo(&self) -> Result<RunDemoOutcome, CompositionError> {
        let run = self
            .runner
            .start(
                self.request_context.clone(),
                self.workflow_id.clone(),
                WorkflowVersion::V1,
                RUN_KEY.to_owned(),
                "{}".to_owned(),
            )
            .await
            .map_err(|_| CompositionError::OperationFailed)?;

        let outcome = self
            .evaluation
            .evaluate_and_store(TENANT, run.id.as_str(), &self.evaluation_definition)
            .await
            .map_err(|_| CompositionError::OperationFailed)?;
        let result = match outcome {
            CreateOrMatch::Created(result) | CreateOrMatch::Existing(result) => result,
            CreateOrMatch::Conflict => return Err(CompositionError::OperationFailed),
        };

        emit_telemetry(&self.local_telemetry, &result)
            .map_err(|_| CompositionError::OperationFailed)?;

        Ok(RunDemoOutcome {
            run_id: result.logical_key.workflow_run_id.clone(),
            evidence_digest: result.evidence_digest.clone(),
            result_digest: result.content_hash.clone(),
            verdict: verdict_name(result.verdict),
        })
    }

    /// Tenant-fixed lookup of one stored run by id. Returns `Ok(None)` for a
    /// missing run or a run belonging to a different tenant; never synthesizes
    /// a result.
    pub fn get_run(&self, run_id: &str) -> Result<Option<RunView>, CompositionError> {
        if run_id.is_empty() {
            return Err(CompositionError::InvalidRequest);
        }
        if run_id.len() > MAX_RUN_ID_BYTES {
            return Err(CompositionError::LimitExceeded);
        }
        let run_id = LogicalId::new(run_id).map_err(|_| CompositionError::InvalidRequest)?;
        let Some(run) = self
            .workflow_store
            .get(&self.tenant, &run_id)
            .map_err(|_| CompositionError::OperationFailed)?
        else {
            return Ok(None);
        };
        let summary = RunSummary::from(&run);
        Ok(Some(RunView {
            run_id: summary.id.as_str().to_owned(),
            status: status_name(summary.status),
            terminal_reason: summary.terminal_reason.map(reason_name),
            output: run.attempt.and_then(|attempt| attempt.result),
        }))
    }

    /// Tenant-fixed bounded telemetry query returning only allowlisted
    /// attributes, most recent first.
    pub fn query_telemetry(
        &self,
        limit: usize,
    ) -> Result<Vec<TelemetryAttributes>, CompositionError> {
        if limit == 0 {
            return Err(CompositionError::InvalidRequest);
        }
        if limit > MAX_TELEMETRY_QUERY_LIMIT {
            return Err(CompositionError::LimitExceeded);
        }
        let query = TelemetryQueryV1::new(limit).map_err(|_| CompositionError::InvalidRequest)?;
        let context = TelemetryContext::new(
            observability::TenantId::new(TENANT).map_err(|_| CompositionError::OperationFailed)?,
        );
        let records = self
            .telemetry
            .query(&context, &query)
            .map_err(|_| CompositionError::OperationFailed)?;
        Ok(records.iter().map(project_attributes).collect())
    }
}

fn project_attributes(record: &TelemetryRecordV1) -> TelemetryAttributes {
    let attributes = &record.envelope.event.attributes;
    TelemetryAttributes {
        workflow_run_id: attributes.get("workflow_run_id").cloned(),
        evidence_digest: attributes.get("evidence_digest").cloned(),
        result_digest: attributes.get("result_digest").cloned(),
        verdict: attributes.get("verdict").cloned(),
    }
}

fn agent_definition(id: agent::AgentId) -> agent::AgentDefinitionV1 {
    agent::AgentDefinitionV1 {
        version: agent::DefinitionVersion::V1,
        id,
        name: "Deterministic demo agent".to_owned(),
        description: "Returns one static bounded result".to_owned(),
        model: agent::ModelPolicy {
            reference: "static.demo".to_owned(),
        },
        instructions: "Return the configured deterministic response.".to_owned(),
        skills: vec![],
        steering: vec![],
        allowed_tool_ids: vec![],
        memory: agent::MemoryPolicy {
            enabled: false,
            max_items: 0,
        },
        knowledge: agent::KnowledgePolicy {
            enabled: false,
            namespace: "disabled".to_owned(),
            max_results: 0,
        },
        sandbox: agent::SandboxPolicy {
            allow_execution: false,
        },
        communication: agent::CommunicationPolicy {
            allow_messages: false,
        },
        limits: agent::ExecutionLimits {
            max_tool_calls: 1,
            max_output_bytes: 1024,
        },
    }
}

fn emit_telemetry(
    local: &observability::local::LocalTelemetry,
    result: &EvaluationResultV1,
) -> Result<(), observability::ObservabilityError> {
    let telemetry = TelemetryService::new(
        local.clone(),
        local.clone(),
        FixedClock,
        MAX_TELEMETRY_QUERY_LIMIT,
    )?;
    let mut attributes = BTreeMap::new();
    attributes.insert(
        "workflow_run_id".to_owned(),
        result.logical_key.workflow_run_id.clone(),
    );
    attributes.insert("evidence_digest".to_owned(), result.evidence_digest.clone());
    attributes.insert("result_digest".to_owned(), result.content_hash.clone());
    attributes.insert(
        "verdict".to_owned(),
        verdict_name(result.verdict).to_owned(),
    );
    let event = TelemetryEventV1::new(
        EventName::new("workflow_evaluated")?,
        EventTarget::new("workflow_evaluation_demo")?,
        Severity::Info,
        "workflow evaluation completed",
        attributes,
    )?;
    telemetry.emit(
        &TelemetryContext::new(observability::TenantId::new(TENANT)?),
        event,
    )
}

const fn verdict_name(verdict: Verdict) -> &'static str {
    match verdict {
        Verdict::Pass => "pass",
        Verdict::Fail => "fail",
        Verdict::Error => "error",
    }
}

const fn status_name(status: workflow::RunStatus) -> &'static str {
    match status {
        workflow::RunStatus::Pending => "pending",
        workflow::RunStatus::Running => "running",
        workflow::RunStatus::Succeeded => "succeeded",
        workflow::RunStatus::Failed => "failed",
        workflow::RunStatus::Cancelled => "cancelled",
    }
}

const fn reason_name(reason: workflow::TerminalReason) -> &'static str {
    match reason {
        workflow::TerminalReason::Completed => "completed",
        workflow::TerminalReason::InvocationFailed => "invocation_failed",
        workflow::TerminalReason::Cancelled => "cancelled",
    }
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
