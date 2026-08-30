use super::*;
use crate::{AgentInvocationRequest, InvocationEvidence, InvocationEvidenceSink};
use agent::{
    InvocationModelEvidence, InvocationModelFinishReason, InvocationModelIdempotency,
    InvocationModelTokenUsage,
};
use policy::{
    CorrelationId, GrantV1, PrincipalId, RequestId, TenantId, allow_decision, deny_decision,
};
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

fn ready<T>(mut future: Pin<Box<dyn Future<Output = T> + Send + '_>>) -> T {
    match future
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()))
    {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("deterministic fixture unexpectedly pending"),
    }
}

fn trusted(tenant: &str) -> TrustedContextV1 {
    TrustedContextV1 {
        tenant_id: TenantId::new(tenant).expect("tenant"),
        principal_id: PrincipalId::new("principal").expect("principal"),
        request_id: RequestId::new("request").expect("request"),
        correlation_id: CorrelationId::new("correlation").expect("correlation"),
    }
}

#[derive(Clone)]
struct Source(Result<TrustedContextV1, WorkflowError>);
impl TrustedContextSource for Source {
    fn resolve(&self) -> std::result::Result<TrustedContextV1, WorkflowError> {
        self.0.clone()
    }
}

#[derive(Clone)]
struct Policy {
    allow: bool,
    memory_enabled: bool,
    calls: Arc<Mutex<Vec<CapabilityV1>>>,
}
impl PolicyResolver for Policy {
    fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
        self.calls
            .lock()
            .expect("policy calls")
            .push(request.capability);
        if self.allow {
            allow_decision(
                &request,
                &GrantV1::new(
                    ["read".to_owned()],
                    self.memory_enabled,
                    false,
                    false,
                    false,
                )
                .expect("grant"),
            )
            .expect("decision")
        } else {
            deny_decision()
        }
    }
}

#[derive(Clone, Default)]
struct Domain(Arc<Mutex<Vec<&'static str>>>);
impl Domain {
    fn record(&self, value: &'static str) {
        self.0.lock().expect("calls").push(value);
    }
    fn calls(&self) -> Vec<&'static str> {
        self.0.lock().expect("calls").clone()
    }
}
impl WorkflowStore for Domain {
    fn create_or_return(
        &self,
        _: crate::StartIdentity,
        _: crate::Run,
    ) -> std::result::Result<crate::CreateRun, WorkflowError> {
        self.record("store.create");
        Err(WorkflowError::AdapterFailure)
    }
    fn get(
        &self,
        _: &LogicalId,
        _: &LogicalId,
    ) -> std::result::Result<Option<crate::Run>, WorkflowError> {
        self.record("store.get");
        Err(WorkflowError::AdapterFailure)
    }
    fn list(&self, _: &LogicalId) -> std::result::Result<Vec<crate::Run>, WorkflowError> {
        self.record("store.list");
        Err(WorkflowError::AdapterFailure)
    }
    fn transition(
        &self,
        _: &LogicalId,
        _: &LogicalId,
        _: u64,
        _: crate::RunStatus,
        _: crate::Transition,
    ) -> std::result::Result<crate::TransitionResult, WorkflowError> {
        self.record("store.transition");
        Err(WorkflowError::AdapterFailure)
    }
}
impl WorkflowDefinitionCatalog for Domain {
    fn resolve(
        &self,
        _: &LogicalId,
        _: WorkflowVersion,
    ) -> std::result::Result<Option<WorkflowDefinitionV1>, WorkflowError> {
        self.record("catalog.resolve");
        Err(WorkflowError::AdapterFailure)
    }
}
impl CeilingAgentRuntime for Domain {
    fn validate_agent(&self, _: &AgentId) -> std::result::Result<bool, WorkflowError> {
        self.record("runtime.validate");
        Err(WorkflowError::AdapterFailure)
    }
    fn invoke_with_ceiling<'a>(
        &'a self,
        _: CeilingAgentInvocation,
        _: llm_gateway::InvocationControl<'a>,
    ) -> CeilingInvocationFuture<'a> {
        self.record("runtime.invoke");
        Box::pin(async { Err(WorkflowError::AdapterFailure) })
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

type TestMcp = WorkflowMcp<Domain, Domain, Domain, Source, Policy>;
type TestService = (TestMcp, Domain, Arc<Mutex<Vec<CapabilityV1>>>);

fn service(source: Result<TrustedContextV1, WorkflowError>, allow: bool) -> TestService {
    let domain = Domain::default();
    let calls = Arc::new(Mutex::new(Vec::new()));
    let resolver = WorkflowPolicyContextResolver::new(
        Source(source),
        Policy {
            allow,
            memory_enabled: false,
            calls: Arc::clone(&calls),
        },
    );
    (
        WorkflowMcp::new(
            domain.clone(),
            domain.clone(),
            domain.clone(),
            resolver,
            Box::new(Factory),
        ),
        domain,
        calls,
    )
}

#[test]
fn mcp_dtos_reject_caller_identity_and_unknown_fields() {
    assert_eq!(tool_names(), WORKFLOW_TOOLS);
    assert!(
        serde_json::from_value::<StartInput>(json!({
            "workflow_id":"workflow","run_key":"key","input":"{}","tenant_id":"forged"
        }))
        .is_err()
    );
    assert!(
        serde_json::from_value::<RunIdInput>(json!({"run_id":"run","principal_id":"forged"}))
            .is_err()
    );
}

#[test]
fn public_mcp_errors_hide_adapter_and_secret_details() {
    assert_eq!(
        tool_response(Err(public_error(WorkflowError::AdapterFailure))),
        r#"{"error":"operation_failed"}"#
    );
    assert_eq!(
        tool_response(Err(public_error(WorkflowError::NotFound))),
        r#"{"error":"not_found"}"#
    );
}

#[test]
fn source_failure_and_policy_deny_are_pre_domain() {
    let (failed, domain, calls) = service(Err(WorkflowError::AdapterFailure), true);
    assert_eq!(
        tool_response(failed.get_json(RunIdInput {
            run_id: "run".to_owned()
        })),
        r#"{"error":"operation_failed"}"#
    );
    assert!(domain.calls().is_empty());
    assert!(calls.lock().expect("calls").is_empty());

    let (denied, domain, calls) = service(Ok(trusted("tenant")), false);
    assert_eq!(
        tool_response(denied.list_json()),
        r#"{"error":"not_found"}"#
    );
    assert!(domain.calls().is_empty());
    assert_eq!(
        calls.lock().expect("calls").as_slice(),
        &[CapabilityV1::WorkflowList]
    );
}

#[test]
fn invalid_start_and_validate_are_pre_policy_and_pre_domain() {
    let (service, domain, calls) = service(Ok(trusted("tenant")), true);
    assert_eq!(
        service
            .validate_json(WorkflowDefinitionInput {
                id: "workflow".to_owned(),
                agent_id: "agent".to_owned(),
                max_input_bytes: 1,
                max_evidence_bytes: 1,
            })
            .expect("response"),
        r#"{"error":"invalid_definition","valid":false}"#
    );
    assert!(
        ready(Box::pin(service.start_json(StartInput {
            workflow_id: "Invalid".to_owned(),
            run_key: "key".to_owned(),
            input: "{}".to_owned(),
        })))
        .is_err()
    );
    assert!(domain.calls().is_empty());
    assert!(calls.lock().expect("calls").is_empty());
}

#[test]
fn allow_uses_exact_capability_before_each_domain_path() {
    let operations = [
        (CapabilityV1::WorkflowGet, "get"),
        (CapabilityV1::WorkflowList, "list"),
        (CapabilityV1::WorkflowCancel, "cancel"),
    ];
    for (capability, operation) in operations {
        let (service, domain, calls) = service(Ok(trusted("tenant")), true);
        match operation {
            "get" => {
                let _ = service.get_json(RunIdInput {
                    run_id: "run".to_owned(),
                });
            }
            "list" => {
                let _ = service.list_json();
            }
            "cancel" => {
                let _ = service.cancel_json(RunIdInput {
                    run_id: "run".to_owned(),
                });
            }
            _ => unreachable!(),
        }
        assert_eq!(calls.lock().expect("calls").as_slice(), &[capability]);
        assert!(!domain.calls().is_empty());
    }
}

fn model_evidence() -> InvocationModelEvidence {
    InvocationModelEvidence {
        provider_id: "provider\"\\\n".to_owned(),
        model_id: "model".to_owned(),
        provider_request_id: Some("request".to_owned()),
        finish_reason: InvocationModelFinishReason::ToolCalls,
        token_usage: Some(InvocationModelTokenUsage {
            input_tokens: 2,
            output_tokens: 3,
            total_tokens: 5,
        }),
        idempotency: InvocationModelIdempotency::Accepted,
    }
}

#[test]
fn canonical_model_evidence_has_exact_order_fields_nulls_and_escaping() {
    assert_eq!(
        canonical_model_evidence(&model_evidence()).expect("evidence"),
        r#"{"finish_reason":"tool_calls","idempotency":"accepted","model_id":"model","provider_id":"provider\"\\\n","provider_request_id":"request","token_usage":{"input_tokens":2,"output_tokens":3,"total_tokens":5}}"#
    );
    let mut absent = model_evidence();
    absent.provider_request_id = None;
    absent.token_usage = None;
    assert!(
        canonical_model_evidence(&absent)
            .expect("evidence")
            .ends_with(r#""provider_request_id":null,"token_usage":null}"#)
    );
}

#[derive(Default)]
struct Sink(Vec<InvocationEvidence>);
impl InvocationEvidenceSink for Sink {
    fn emit(&mut self, evidence: InvocationEvidence) -> std::result::Result<(), WorkflowError> {
        self.0.push(evidence);
        Ok(())
    }
}

#[derive(Clone)]
struct Runtime {
    calls: Arc<Mutex<Vec<(CeilingAgentInvocation, String, Instant)>>>,
}
impl CeilingAgentRuntime for Runtime {
    fn validate_agent(&self, _: &AgentId) -> std::result::Result<bool, WorkflowError> {
        Ok(true)
    }
    fn invoke_with_ceiling<'a>(
        &'a self,
        invocation: CeilingAgentInvocation,
        control: llm_gateway::InvocationControl<'a>,
    ) -> CeilingInvocationFuture<'a> {
        self.calls.lock().expect("calls").push((
            invocation,
            control.idempotency_key.as_str().to_owned(),
            control.deadline.instant(),
        ));
        Box::pin(async {
            Ok(agent::InvocationResult {
                capability_scope_digest: "scope".to_owned(),
                events: vec![],
                output: "output".to_owned(),
                model_evidence: model_evidence(),
            })
        })
    }
}

fn invocation_request(digest: String) -> AgentInvocationRequest {
    AgentInvocationRequest {
        context: crate::RequestContext {
            tenant_id: LogicalId::new("tenant").expect("tenant"),
            principal_id: LogicalId::new("principal").expect("principal"),
            request_id: LogicalId::new("request").expect("request"),
            correlation_id: LogicalId::new("correlation").expect("correlation"),
        },
        agent_id: AgentId::new("agent").expect("agent"),
        input: "input".to_owned(),
        attempt_id: LogicalId::new("attempt").expect("attempt"),
        effective_capability_ceiling: EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: vec!["read".to_owned()],
            memory_enabled: false,
            knowledge_enabled: false,
            sandbox_execution_allowed: false,
            communication_allowed: false,
        },
        policy_decision_digest: digest,
    }
}

#[test]
fn policy_invoker_rejects_bad_evidence_pre_runtime_and_forwards_control_unchanged() {
    let calls = Arc::new(Mutex::new(Vec::new()));
    let invoker = PolicyAwareAgentInvoker::new(Runtime {
        calls: Arc::clone(&calls),
    });
    let signal = crate::CancellationSignal::new();
    let deadline = Deadline(Instant::now() + Duration::from_secs(1));
    let key = llm_gateway::IdempotencyKey::new("stable-key").expect("key");
    let control = llm_gateway::InvocationControl {
        idempotency_key: &key,
        cancellation: &signal,
        deadline: &deadline,
    };
    let mut sink = Sink::default();
    assert_eq!(
        ready(invoker.invoke(invocation_request("bad".to_owned()), control, &mut sink)),
        Err(WorkflowError::InvalidRequest)
    );
    assert!(calls.lock().expect("calls").is_empty());
    assert!(sink.0.is_empty());

    let result = ready(invoker.invoke(invocation_request("a".repeat(64)), control, &mut sink))
        .expect("invoke");
    assert_eq!(result.capability_scope_digest, "scope");
    assert_eq!(
        sink.0
            .iter()
            .map(|item| item.kind.as_str())
            .collect::<Vec<_>>(),
        ["llm_generation", "result"]
    );
    let calls = calls.lock().expect("calls");
    assert_eq!(calls[0].0.context.tenant_id().as_str(), "tenant");
    assert_eq!(calls[0].0.context.principal_id().as_str(), "principal");
    assert_eq!(calls[0].0.context.request_id().as_str(), "request");
    assert_eq!(calls[0].0.context.correlation_id().as_str(), "correlation");
    assert_eq!(
        calls[0].0.effective_capability_ceiling.allowed_tool_ids,
        ["read"]
    );
    assert_eq!(calls[0].1, "stable-key");
    assert_eq!(calls[0].2, deadline.0);
    let evidence = sink
        .0
        .iter()
        .map(|item| item.data.as_str())
        .collect::<String>();
    for private_context_value in ["tenant", "principal", "correlation"] {
        assert!(!evidence.contains(private_context_value));
    }
}

#[test]
fn ceiling_runtime_trait_is_object_safe() {
    let runtime = Runtime {
        calls: Arc::new(Mutex::new(Vec::new())),
    };
    let object: &dyn CeilingAgentRuntime = &runtime;
    assert!(
        object
            .validate_agent(&AgentId::new("agent").expect("agent"))
            .expect("validate")
    );
}

#[cfg(feature = "memory")]
mod composition {
    use super::*;
    use crate::memory::{InMemoryWorkflowStore, StaticWorkflowCatalog};
    use agent::{
        AgentDefinitionV1, AgentRegistry, CommunicationPolicy, DefinitionVersion, DenySandbox,
        ExecutionLimits, InMemoryDefinitionStore, InvocationContextV1, KnowledgePolicy,
        LocalAgentRuntime, MemoryPolicy, MemoryRequest, MemoryStore, ModelPolicy, SandboxPolicy,
        StaticReferenceCatalog, ToolDescriptor, ToolRegistry, ToolRequest,
    };
    use llm_gateway::{
        FinishReason, IdempotencyDisposition, JsonObject, ProviderRequestId, TokenUsage, ToolCall,
        ToolName,
        r#static::{StaticFixture, StaticProvider},
    };
    use std::collections::BTreeMap;

    #[derive(Default)]
    struct Tools(Arc<Mutex<Vec<String>>>);
    impl ToolRegistry for Tools {
        fn resolve(&self, id: &str) -> Result<ToolDescriptor, agent::DefinitionError> {
            Ok(ToolDescriptor { id: id.to_owned() })
        }
        fn invoke(
            &self,
            tool: &ToolDescriptor,
            _: ToolRequest,
        ) -> Result<String, agent::DefinitionError> {
            self.0.lock().expect("tools").push(tool.id.clone());
            Ok("tool-output".to_owned())
        }
    }

    #[derive(Clone, Default)]
    struct TenantMemory {
        values: Arc<Mutex<BTreeMap<String, Vec<String>>>>,
        contexts: Arc<Mutex<Vec<InvocationContextV1>>>,
    }
    impl MemoryStore for TenantMemory {
        fn recall(&self, request: MemoryRequest) -> Result<Vec<String>, agent::DefinitionError> {
            self.contexts
                .lock()
                .expect("memory contexts")
                .push(request.context.clone());
            Ok(self
                .values
                .lock()
                .expect("memory values")
                .get(request.context.tenant_id().as_str())
                .cloned()
                .unwrap_or_default())
        }

        fn write(
            &self,
            request: MemoryRequest,
            value: String,
        ) -> Result<(), agent::DefinitionError> {
            self.contexts
                .lock()
                .expect("memory contexts")
                .push(request.context.clone());
            self.values
                .lock()
                .expect("memory values")
                .entry(request.context.tenant_id().as_str().to_owned())
                .or_default()
                .push(value);
            Ok(())
        }
    }

    struct RecordingProvider {
        inner: StaticProvider,
        requests: Arc<Mutex<Vec<String>>>,
    }
    impl llm_gateway::LlmProvider for RecordingProvider {
        fn generate<'a>(
            &'a self,
            request: &'a llm_gateway::GenerateRequest,
            control: llm_gateway::InvocationControl<'a>,
        ) -> llm_gateway::ProviderFuture<'a> {
            self.requests
                .lock()
                .expect("provider requests")
                .push(format!("{request:?}"));
            llm_gateway::LlmProvider::generate(&self.inner, request, control)
        }
    }

    struct LocalRuntime {
        registry: AgentRegistry<InMemoryDefinitionStore, StaticReferenceCatalog>,
        provider: RecordingProvider,
        tools: Tools,
        memory: TenantMemory,
        knowledge: knowledge::r#static::StaticKnowledgeIndex,
        sandbox: DenySandbox,
    }
    impl CeilingAgentRuntime for LocalRuntime {
        fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError> {
            self.registry
                .get(id)
                .map(|_| true)
                .map_err(WorkflowError::from)
        }
        fn invoke_with_ceiling<'a>(
            &'a self,
            invocation: CeilingAgentInvocation,
            control: llm_gateway::InvocationControl<'a>,
        ) -> CeilingInvocationFuture<'a> {
            Box::pin(async move {
                LocalAgentRuntime::new(
                    &self.registry,
                    &self.provider,
                    &self.tools,
                    &self.memory,
                    &self.knowledge,
                    &self.sandbox,
                )
                .invoke_with_ceiling(
                    invocation.context,
                    &invocation.agent_id,
                    invocation.input,
                    &invocation.effective_capability_ceiling,
                    control,
                )
                .await
                .map_err(WorkflowError::from)
            })
        }
    }

    #[derive(Clone)]
    struct DynamicSource(Arc<Mutex<TrustedContextV1>>);
    impl TrustedContextSource for DynamicSource {
        fn resolve(&self) -> Result<TrustedContextV1, WorkflowError> {
            Ok(self.0.lock().expect("trusted context").clone())
        }
    }

    #[test]
    fn workflow_composes_real_agent_and_gateway_with_narrowed_tools() {
        let definition = AgentDefinitionV1 {
            version: DefinitionVersion::V1,
            id: AgentId::new("agent").expect("agent"),
            name: "Agent".to_owned(),
            description: "composition fixture".to_owned(),
            model: ModelPolicy {
                reference: "provider.model".to_owned(),
            },
            instructions: "Use read only".to_owned(),
            skills: vec![],
            steering: vec![],
            allowed_tool_ids: vec!["read".to_owned(), "write".to_owned()],
            memory: MemoryPolicy {
                enabled: false,
                max_items: 0,
            },
            knowledge: KnowledgePolicy {
                enabled: false,
                namespace: "default".to_owned(),
                max_results: 0,
            },
            sandbox: SandboxPolicy {
                allow_execution: false,
            },
            communication: CommunicationPolicy {
                allow_messages: false,
            },
            limits: ExecutionLimits {
                max_tool_calls: 1,
                max_output_bytes: 1024,
            },
        };
        let registry = AgentRegistry::new(
            vec![definition],
            InMemoryDefinitionStore::default(),
            StaticReferenceCatalog::new(
                ["provider.model".to_owned()],
                [],
                [],
                ["read".to_owned(), "write".to_owned()],
            ),
        )
        .expect("registry");
        let fixture = StaticFixture::new(
            "done",
            vec![
                ToolCall::new(
                    ToolName::new("read").expect("name"),
                    JsonObject::new(r#"{"input":"value"}"#).expect("arguments"),
                )
                .expect("call"),
            ],
            Some(ProviderRequestId::new("request").expect("request")),
            FinishReason::ToolCalls,
            Some(TokenUsage::new(1, 1, Some(2)).expect("usage")),
            IdempotencyDisposition::Accepted,
        )
        .expect("fixture");
        let tools = Tools::default();
        let runtime = LocalRuntime {
            registry,
            provider: RecordingProvider {
                inner: StaticProvider::success(fixture),
                requests: Arc::new(Mutex::new(Vec::new())),
            },
            tools: Tools(Arc::clone(&tools.0)),
            memory: TenantMemory::default(),
            knowledge: knowledge::r#static::StaticKnowledgeIndex::new(vec![]).expect("knowledge"),
            sandbox: DenySandbox,
        };
        let store = InMemoryWorkflowStore::default();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mcp = WorkflowMcp::new(
            store,
            StaticWorkflowCatalog::new([WorkflowDefinitionV1 {
                id: LogicalId::new("workflow").expect("workflow"),
                version: WorkflowVersion::V1,
                step: AgentStep {
                    agent_id: AgentId::new("agent").expect("agent"),
                },
                budget: WorkflowBudget::default(),
            }]),
            runtime,
            WorkflowPolicyContextResolver::new(
                Source(Ok(trusted("tenant"))),
                Policy {
                    allow: true,
                    memory_enabled: false,
                    calls,
                },
            ),
            Box::new(Factory),
        );
        let response = ready(Box::pin(mcp.start_json(StartInput {
            workflow_id: "workflow".to_owned(),
            run_key: "key".to_owned(),
            input: "{}".to_owned(),
        })))
        .expect("start");
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&response).expect("json")["status"],
            "succeeded"
        );
        assert_eq!(tools.0.lock().expect("tools").as_slice(), &["read"]);
    }

    #[test]
    fn workflow_routes_trusted_tenants_to_isolated_agent_memory_contexts() {
        let definition = AgentDefinitionV1 {
            version: DefinitionVersion::V1,
            id: AgentId::new("memory-agent").expect("agent"),
            name: "Memory agent".to_owned(),
            description: "tenant context fixture".to_owned(),
            model: ModelPolicy {
                reference: "provider.model".to_owned(),
            },
            instructions: "Write memory".to_owned(),
            skills: vec![],
            steering: vec![],
            allowed_tool_ids: vec![],
            memory: MemoryPolicy {
                enabled: true,
                max_items: 1,
            },
            knowledge: KnowledgePolicy {
                enabled: false,
                namespace: "default".to_owned(),
                max_results: 0,
            },
            sandbox: SandboxPolicy {
                allow_execution: false,
            },
            communication: CommunicationPolicy {
                allow_messages: false,
            },
            limits: ExecutionLimits {
                max_tool_calls: 1,
                max_output_bytes: 1024,
            },
        };
        let registry = AgentRegistry::new(
            vec![definition],
            InMemoryDefinitionStore::default(),
            StaticReferenceCatalog::new(["provider.model".to_owned()], [], [], []),
        )
        .expect("registry");
        let fixture = StaticFixture::new(
            "done",
            vec![
                ToolCall::new(
                    ToolName::new("factory.memory.write").expect("name"),
                    JsonObject::new(r#"{"value":"tenant-value"}"#).expect("arguments"),
                )
                .expect("call"),
            ],
            Some(ProviderRequestId::new("provider-request").expect("request")),
            FinishReason::ToolCalls,
            Some(TokenUsage::new(1, 1, Some(2)).expect("usage")),
            IdempotencyDisposition::Accepted,
        )
        .expect("fixture");
        let memory = TenantMemory::default();
        let provider_requests = Arc::new(Mutex::new(Vec::new()));
        let runtime = LocalRuntime {
            registry,
            provider: RecordingProvider {
                inner: StaticProvider::success(fixture),
                requests: Arc::clone(&provider_requests),
            },
            tools: Tools::default(),
            memory: memory.clone(),
            knowledge: knowledge::r#static::StaticKnowledgeIndex::new(vec![]).expect("knowledge"),
            sandbox: DenySandbox,
        };
        let tenant_a = TrustedContextV1 {
            tenant_id: TenantId::new("tenant-a").expect("tenant"),
            principal_id: PrincipalId::new("principal-a").expect("principal"),
            request_id: RequestId::new("request-a").expect("request"),
            correlation_id: CorrelationId::new("correlation-a").expect("correlation"),
        };
        let tenant_b = TrustedContextV1 {
            tenant_id: TenantId::new("tenant-b").expect("tenant"),
            principal_id: PrincipalId::new("principal-b").expect("principal"),
            request_id: RequestId::new("request-b").expect("request"),
            correlation_id: CorrelationId::new("correlation-b").expect("correlation"),
        };
        let trusted_context = Arc::new(Mutex::new(tenant_a.clone()));
        let store = InMemoryWorkflowStore::default();
        let mcp = WorkflowMcp::new(
            store.clone(),
            StaticWorkflowCatalog::new([WorkflowDefinitionV1 {
                id: LogicalId::new("memory-workflow").expect("workflow"),
                version: WorkflowVersion::V1,
                step: AgentStep {
                    agent_id: AgentId::new("memory-agent").expect("agent"),
                },
                budget: WorkflowBudget::default(),
            }]),
            runtime,
            WorkflowPolicyContextResolver::new(
                DynamicSource(Arc::clone(&trusted_context)),
                Policy {
                    allow: true,
                    memory_enabled: true,
                    calls: Arc::new(Mutex::new(Vec::new())),
                },
            ),
            Box::new(Factory),
        );

        for (context, run_key) in [(tenant_a.clone(), "run-a"), (tenant_b.clone(), "run-b")] {
            *trusted_context.lock().expect("trusted context") = context;
            let response = ready(Box::pin(mcp.start_json(StartInput {
                workflow_id: "memory-workflow".to_owned(),
                run_key: run_key.to_owned(),
                input: "{}".to_owned(),
            })))
            .expect("start");
            assert_eq!(
                serde_json::from_str::<serde_json::Value>(&response).expect("json")["status"],
                "succeeded"
            );
        }

        let provider_payloads = provider_requests.lock().expect("provider requests");
        assert_eq!(provider_payloads.len(), 2);
        for payload in provider_payloads.iter() {
            for context_value in [
                "tenant-a",
                "tenant-b",
                "principal-a",
                "principal-b",
                "request-a",
                "request-b",
                "correlation-a",
                "correlation-b",
            ] {
                assert!(!payload.contains(context_value));
            }
        }
        drop(provider_payloads);

        let values = memory.values.lock().expect("memory values");
        assert_eq!(
            values.get("tenant-a"),
            Some(&vec!["tenant-value".to_owned()])
        );
        assert_eq!(
            values.get("tenant-b"),
            Some(&vec!["tenant-value".to_owned()])
        );
        drop(values);
        let contexts = memory.contexts.lock().expect("memory contexts");
        assert_eq!(
            contexts.as_slice(),
            &[
                agent::InvocationContextV1::new(
                    agent::TenantId::new("tenant-a").expect("tenant"),
                    agent::PrincipalId::new("principal-a").expect("principal"),
                    agent::RequestId::new("request-a").expect("request"),
                    agent::CorrelationId::new("correlation-a").expect("correlation"),
                ),
                agent::InvocationContextV1::new(
                    agent::TenantId::new("tenant-b").expect("tenant"),
                    agent::PrincipalId::new("principal-b").expect("principal"),
                    agent::RequestId::new("request-b").expect("request"),
                    agent::CorrelationId::new("correlation-b").expect("correlation"),
                )
            ]
        );
        drop(contexts);

        for tenant in ["tenant-a", "tenant-b"] {
            let runs = store
                .list(&LogicalId::new(tenant).expect("tenant"))
                .expect("runs");
            assert_eq!(runs.len(), 1);
            let evidence = runs[0]
                .events
                .iter()
                .map(|event| event.data.as_str())
                .collect::<String>();
            for context_value in [
                "tenant-a",
                "tenant-b",
                "principal-a",
                "principal-b",
                "request-a",
                "request-b",
                "correlation-a",
                "correlation-b",
            ] {
                assert!(!evidence.contains(context_value));
            }
        }
    }

    #[derive(Debug, Eq, PartialEq)]
    struct KnowledgeSearchObservation {
        tenant_id: String,
        principal_id: String,
        namespace: String,
        query: String,
        limit: u32,
    }

    struct RecordingKnowledgeIndex {
        inner: knowledge::r#static::StaticKnowledgeIndex,
        searches: Arc<Mutex<Vec<KnowledgeSearchObservation>>>,
    }
    impl knowledge::KnowledgeIndex for RecordingKnowledgeIndex {
        fn search(
            &self,
            request: &knowledge::SearchRequest,
        ) -> Result<Vec<knowledge::KnowledgeDocument>, knowledge::KnowledgeError> {
            self.searches
                .lock()
                .expect("knowledge searches")
                .push(KnowledgeSearchObservation {
                    tenant_id: request.context().tenant_id().as_str().to_owned(),
                    principal_id: request.context().principal_id().as_str().to_owned(),
                    namespace: request.namespace().as_str().to_owned(),
                    query: request.query().as_str().to_owned(),
                    limit: request.limit().get(),
                });
            knowledge::KnowledgeIndex::search(&self.inner, request)
        }
    }

    struct KnowledgeRuntime {
        registry: AgentRegistry<InMemoryDefinitionStore, StaticReferenceCatalog>,
        provider: StaticProvider,
        knowledge: RecordingKnowledgeIndex,
        results: Arc<Mutex<Vec<agent::InvocationResult>>>,
    }
    impl CeilingAgentRuntime for KnowledgeRuntime {
        fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError> {
            self.registry
                .get(id)
                .map(|_| true)
                .map_err(WorkflowError::from)
        }

        fn invoke_with_ceiling<'a>(
            &'a self,
            invocation: CeilingAgentInvocation,
            control: llm_gateway::InvocationControl<'a>,
        ) -> CeilingInvocationFuture<'a> {
            Box::pin(async move {
                let tools = Tools::default();
                let memory = agent::InMemoryMemoryStore::default();
                let sandbox = DenySandbox;
                let result = LocalAgentRuntime::new(
                    &self.registry,
                    &self.provider,
                    &tools,
                    &memory,
                    &self.knowledge,
                    &sandbox,
                )
                .invoke_with_ceiling(
                    invocation.context,
                    &invocation.agent_id,
                    invocation.input,
                    &invocation.effective_capability_ceiling,
                    control,
                )
                .await
                .map_err(WorkflowError::from)?;
                self.results
                    .lock()
                    .expect("agent results")
                    .push(result.clone());
                Ok(result)
            })
        }
    }

    #[derive(Clone)]
    struct KnowledgeGrantPolicy {
        knowledge_enabled: bool,
    }
    impl PolicyResolver for KnowledgeGrantPolicy {
        fn authorize(&self, request: AuthorizationRequestV1) -> AuthorizationDecisionV1 {
            allow_decision(
                &request,
                &GrantV1::new(
                    Vec::<String>::new(),
                    false,
                    self.knowledge_enabled,
                    false,
                    false,
                )
                .expect("knowledge grant"),
            )
            .expect("knowledge decision")
        }
    }

    fn knowledge_agent_definition() -> AgentDefinitionV1 {
        AgentDefinitionV1 {
            version: DefinitionVersion::V1,
            id: AgentId::new("knowledge-agent").expect("agent"),
            name: "Knowledge agent".to_owned(),
            description: "scoped knowledge composition fixture".to_owned(),
            model: ModelPolicy {
                reference: "provider.model".to_owned(),
            },
            instructions: "Search scoped knowledge".to_owned(),
            skills: vec![],
            steering: vec![],
            allowed_tool_ids: vec![],
            memory: MemoryPolicy {
                enabled: false,
                max_items: 0,
            },
            knowledge: KnowledgePolicy {
                enabled: true,
                namespace: "selected-namespace".to_owned(),
                max_results: 2,
            },
            sandbox: SandboxPolicy {
                allow_execution: false,
            },
            communication: CommunicationPolicy {
                allow_messages: false,
            },
            limits: ExecutionLimits {
                max_tool_calls: 1,
                max_output_bytes: 1024,
            },
        }
    }

    fn knowledge_fixture() -> StaticFixture {
        StaticFixture::new(
            "model-output",
            vec![
                ToolCall::new(
                    ToolName::new("factory.knowledge.search").expect("name"),
                    JsonObject::new(r#"{"query":"needle"}"#).expect("arguments"),
                )
                .expect("call"),
            ],
            Some(ProviderRequestId::new("knowledge-request").expect("request")),
            FinishReason::ToolCalls,
            Some(TokenUsage::new(1, 1, Some(2)).expect("usage")),
            IdempotencyDisposition::Accepted,
        )
        .expect("fixture")
    }

    fn document(
        tenant: &str,
        namespace: &str,
        document_id: &str,
        text: &str,
    ) -> knowledge::KnowledgeDocument {
        knowledge::KnowledgeDocument::new(
            knowledge::TenantId::new(tenant).expect("tenant"),
            knowledge::NamespaceId::new(namespace).expect("namespace"),
            knowledge::DocumentId::new(document_id).expect("document"),
            text,
        )
        .expect("document")
    }

    #[test]
    fn workflow_composes_scoped_knowledge_without_persisting_agent_events_or_context() {
        let registry = AgentRegistry::new(
            vec![knowledge_agent_definition()],
            InMemoryDefinitionStore::default(),
            StaticReferenceCatalog::new(["provider.model".to_owned()], [], [], []),
        )
        .expect("registry");
        let searches = Arc::new(Mutex::new(Vec::new()));
        let agent_results = Arc::new(Mutex::new(Vec::new()));
        let runtime = KnowledgeRuntime {
            registry,
            provider: StaticProvider::success(knowledge_fixture()),
            knowledge: RecordingKnowledgeIndex {
                inner: knowledge::r#static::StaticKnowledgeIndex::new(vec![
                    document(
                        "tenant-visible",
                        "selected-namespace",
                        "visible-document",
                        "needle visible text",
                    ),
                    document(
                        "tenant-visible",
                        "other-namespace",
                        "wrong-namespace-document",
                        "needle namespace leak",
                    ),
                    document(
                        "other-tenant",
                        "selected-namespace",
                        "wrong-tenant-document",
                        "needle tenant leak",
                    ),
                ])
                .expect("knowledge"),
                searches: Arc::clone(&searches),
            },
            results: Arc::clone(&agent_results),
        };
        let trusted_context = TrustedContextV1 {
            tenant_id: TenantId::new("tenant-visible").expect("tenant"),
            principal_id: PrincipalId::new("principal-secret").expect("principal"),
            request_id: RequestId::new("request-secret").expect("request"),
            correlation_id: CorrelationId::new("correlation-secret").expect("correlation"),
        };
        let store = InMemoryWorkflowStore::default();
        let mcp = WorkflowMcp::new(
            store.clone(),
            StaticWorkflowCatalog::new([WorkflowDefinitionV1 {
                id: LogicalId::new("knowledge-workflow").expect("workflow"),
                version: WorkflowVersion::V1,
                step: AgentStep {
                    agent_id: AgentId::new("knowledge-agent").expect("agent"),
                },
                budget: WorkflowBudget::default(),
            }]),
            runtime,
            WorkflowPolicyContextResolver::new(
                Source(Ok(trusted_context)),
                KnowledgeGrantPolicy {
                    knowledge_enabled: true,
                },
            ),
            Box::new(Factory),
        );

        let response = ready(Box::pin(mcp.start_json(StartInput {
            workflow_id: "knowledge-workflow".to_owned(),
            run_key: "knowledge-run".to_owned(),
            input: "{}".to_owned(),
        })))
        .expect("start");
        let response_json = serde_json::from_str::<serde_json::Value>(&response).expect("json");
        assert_eq!(response_json["status"], "succeeded");
        assert_eq!(response_json["terminal_reason"], "completed");
        assert!(!response.contains("visible-document"));
        assert!(!response.contains("needle visible text"));

        assert_eq!(
            searches.lock().expect("knowledge searches").as_slice(),
            &[KnowledgeSearchObservation {
                tenant_id: "tenant-visible".to_owned(),
                principal_id: "principal-secret".to_owned(),
                namespace: "selected-namespace".to_owned(),
                query: "needle".to_owned(),
                limit: 2,
            }]
        );
        let results = agent_results.lock().expect("agent results");
        assert_eq!(results.len(), 1);
        assert_eq!(
            results[0].events,
            vec![
                agent::InvocationEvent::ModelInvoked,
                agent::InvocationEvent::KnowledgeSearched {
                    results: vec![agent::KnowledgeResult {
                        document_id: "visible-document".to_owned(),
                        text: "needle visible text".to_owned(),
                    }],
                },
            ]
        );
        drop(results);

        let runs = store
            .list(&LogicalId::new("tenant-visible").expect("tenant"))
            .expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, crate::RunStatus::Succeeded);
        assert_eq!(
            runs[0].terminal_reason,
            Some(crate::TerminalReason::Completed)
        );
        assert_eq!(
            runs[0]
                .events
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            ["started", "llm_generation", "result"]
        );
        assert_eq!(runs[0].events[2].data, "model-output");
        let workflow_evidence = runs[0]
            .events
            .iter()
            .map(|event| event.data.as_str())
            .collect::<String>();
        for private_value in [
            "tenant-visible",
            "principal-secret",
            "request-secret",
            "correlation-secret",
            "selected-namespace",
            "visible-document",
            "needle visible text",
        ] {
            assert!(!workflow_evidence.contains(private_value));
        }
    }

    #[test]
    fn policy_knowledge_ceiling_false_prevents_static_index_effect() {
        let registry = AgentRegistry::new(
            vec![knowledge_agent_definition()],
            InMemoryDefinitionStore::default(),
            StaticReferenceCatalog::new(["provider.model".to_owned()], [], [], []),
        )
        .expect("registry");
        let searches = Arc::new(Mutex::new(Vec::new()));
        let agent_results = Arc::new(Mutex::new(Vec::new()));
        let runtime = KnowledgeRuntime {
            registry,
            provider: StaticProvider::success(knowledge_fixture()),
            knowledge: RecordingKnowledgeIndex {
                inner: knowledge::r#static::StaticKnowledgeIndex::new(vec![document(
                    "tenant-visible",
                    "selected-namespace",
                    "visible-document",
                    "needle visible text",
                )])
                .expect("knowledge"),
                searches: Arc::clone(&searches),
            },
            results: Arc::clone(&agent_results),
        };
        let store = InMemoryWorkflowStore::default();
        let mcp = WorkflowMcp::new(
            store.clone(),
            StaticWorkflowCatalog::new([WorkflowDefinitionV1 {
                id: LogicalId::new("knowledge-workflow").expect("workflow"),
                version: WorkflowVersion::V1,
                step: AgentStep {
                    agent_id: AgentId::new("knowledge-agent").expect("agent"),
                },
                budget: WorkflowBudget::default(),
            }]),
            runtime,
            WorkflowPolicyContextResolver::new(
                Source(Ok(trusted("tenant-visible"))),
                KnowledgeGrantPolicy {
                    knowledge_enabled: false,
                },
            ),
            Box::new(Factory),
        );

        let response = ready(Box::pin(mcp.start_json(StartInput {
            workflow_id: "knowledge-workflow".to_owned(),
            run_key: "denied-knowledge-run".to_owned(),
            input: "{}".to_owned(),
        })))
        .expect("start");
        let response_json = serde_json::from_str::<serde_json::Value>(&response).expect("json");
        assert_eq!(response_json["status"], "failed");
        assert_eq!(response_json["terminal_reason"], "invocation_failed");
        assert!(searches.lock().expect("knowledge searches").is_empty());
        assert!(agent_results.lock().expect("agent results").is_empty());

        let runs = store
            .list(&LogicalId::new("tenant-visible").expect("tenant"))
            .expect("runs");
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].status, crate::RunStatus::Failed);
        assert_eq!(
            runs[0]
                .events
                .iter()
                .map(|event| event.kind.as_str())
                .collect::<Vec<_>>(),
            ["started", "invocation_failed"]
        );
    }
}
