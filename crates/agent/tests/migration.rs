use std::{
    future::Future,
    pin::Pin,
    sync::{Arc, Mutex},
    task::{Context, Poll, Wake, Waker},
    time::Instant,
};

use agent::*;
use llm_gateway::{
    CancellationFuture, CancellationSignal, DeadlineFuture, DeadlineSignal, FinishReason,
    IdempotencyDisposition, IdempotencyKey, InvocationControl, JsonObject, LlmError, LlmProvider,
    ProviderFuture, ToolCall as GatewayToolCall, ToolName,
    r#static::{StaticFixture, StaticProvider},
};

struct NeverCancelled;
impl CancellationSignal for NeverCancelled {
    fn is_cancelled(&self) -> bool {
        false
    }
    fn cancelled(&self) -> CancellationFuture<'_> {
        Box::pin(std::future::pending())
    }
}

struct FutureDeadline;
impl DeadlineSignal for FutureDeadline {
    fn instant(&self) -> Instant {
        Instant::now()
    }
    fn is_elapsed(&self) -> bool {
        false
    }
    fn elapsed(&self) -> DeadlineFuture<'_> {
        Box::pin(std::future::pending())
    }
}

struct Controls {
    key: IdempotencyKey,
    cancellation: NeverCancelled,
    deadline: FutureDeadline,
}
impl Controls {
    fn new() -> Self {
        Self {
            key: IdempotencyKey::new("attempt-1").expect("key"),
            cancellation: NeverCancelled,
            deadline: FutureDeadline,
        }
    }
    fn control(&self) -> InvocationControl<'_> {
        InvocationControl {
            idempotency_key: &self.key,
            cancellation: &self.cancellation,
            deadline: &self.deadline,
        }
    }
}

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}
fn poll_ready<T>(future: impl Future<Output = T>) -> T {
    let waker = Waker::from(Arc::new(NoopWake));
    let mut context = Context::from_waker(&waker);
    let mut future = Box::pin(future);
    match Future::poll(Pin::as_mut(&mut future), &mut context) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("deterministic fixture unexpectedly pending"),
    }
}

fn invocation_context() -> InvocationContextV1 {
    InvocationContextV1::new(
        TenantId::new("tenant").expect("tenant"),
        PrincipalId::new("principal").expect("principal"),
        RequestId::new("request").expect("request"),
        CorrelationId::new("correlation").expect("correlation"),
    )
}

fn definition(tools: Vec<&str>) -> AgentDefinitionV1 {
    AgentDefinitionV1 {
        version: DefinitionVersion::V1,
        id: AgentId::new("agent").expect("id"),
        name: "Agent".to_owned(),
        description: "Migration fixture".to_owned(),
        model: ModelPolicy {
            reference: "static.model".to_owned(),
        },
        instructions: "Respond safely.".to_owned(),
        skills: vec![],
        steering: vec![],
        allowed_tool_ids: tools.into_iter().map(str::to_owned).collect(),
        memory: MemoryPolicy {
            enabled: false,
            max_items: 0,
        },
        knowledge: KnowledgePolicy {
            enabled: false,
            max_results: 0,
        },
        sandbox: SandboxPolicy {
            allow_execution: false,
        },
        communication: CommunicationPolicy {
            allow_messages: false,
        },
        limits: ExecutionLimits {
            max_tool_calls: 4,
            max_output_bytes: 1024,
        },
    }
}

fn fixture(calls: Vec<GatewayToolCall>) -> StaticFixture {
    StaticFixture::new(
        "ok",
        calls,
        None,
        FinishReason::ToolCalls,
        None,
        IdempotencyDisposition::Unsupported,
    )
    .expect("fixture")
}

fn registry(
    definition: AgentDefinitionV1,
) -> AgentRegistry<InMemoryDefinitionStore, StaticReferenceCatalog> {
    AgentRegistry::new(
        vec![definition],
        InMemoryDefinitionStore::default(),
        StaticReferenceCatalog::new(["static.model".to_owned()], [], [], ["allowed".to_owned()]),
    )
    .expect("registry")
}

#[test]
fn gateway_tool_call_is_scope_checked_dispatched_and_projected() {
    let definition = definition(vec!["allowed"]);
    let registry = registry(definition);
    let provider = StaticProvider::success(fixture(vec![
        GatewayToolCall::new(
            ToolName::new("allowed").expect("name"),
            JsonObject::new(r#"{"input":"value"}"#).expect("arguments"),
        )
        .expect("call"),
    ]));
    let tools = FixedToolRegistry::new([("allowed".to_owned(), "tool output".to_owned())]);
    let memory = InMemoryMemoryStore::default();
    let knowledge = StaticKnowledgeStore::default();
    let sandbox = DenySandbox;
    let runtime =
        LocalAgentRuntime::new(&registry, &provider, &tools, &memory, &knowledge, &sandbox);
    let controls = Controls::new();

    let result = poll_ready(runtime.invoke(
        invocation_context(),
        &AgentId::new("agent").expect("id"),
        "request".to_owned(),
        controls.control(),
    ))
    .expect("invocation");

    assert_eq!(result.output, "ok");
    assert_eq!(result.model_evidence.provider_id, "static");
    assert_eq!(result.model_evidence.model_id, "model");
    assert_eq!(result.events.len(), 2);
    assert!(matches!(
        &result.events[1],
        InvocationEvent::ToolCompleted { tool_id, output }
            if tool_id == "allowed" && output == "tool output"
    ));
}

#[test]
fn reserved_memory_tool_is_agent_owned_and_strict() {
    let mut definition = definition(vec![]);
    definition.memory = MemoryPolicy {
        enabled: true,
        max_items: 1,
    };
    let registry = registry(definition);
    let provider = StaticProvider::success(fixture(vec![
        GatewayToolCall::new(
            ToolName::new("factory.memory.write").expect("name"),
            JsonObject::new(r#"{"value":"remember"}"#).expect("arguments"),
        )
        .expect("call"),
    ]));
    let tools = FixedToolRegistry::default();
    let memory = InMemoryMemoryStore::default();
    let knowledge = StaticKnowledgeStore::default();
    let sandbox = DenySandbox;
    let runtime =
        LocalAgentRuntime::new(&registry, &provider, &tools, &memory, &knowledge, &sandbox);
    let controls = Controls::new();

    let result = poll_ready(runtime.invoke(
        invocation_context(),
        &AgentId::new("agent").expect("id"),
        String::new(),
        controls.control(),
    ))
    .expect("invocation");
    assert!(matches!(result.events[1], InvocationEvent::MemoryWritten));
}

struct CountingProvider(Mutex<u32>);
impl LlmProvider for CountingProvider {
    fn generate<'a>(
        &'a self,
        _: &'a llm_gateway::GenerateRequest,
        _: InvocationControl<'a>,
    ) -> ProviderFuture<'a> {
        *self.0.lock().expect("calls") += 1;
        Box::pin(async { Err(LlmError::ProviderRejected) })
    }
}

#[test]
fn malformed_v1_model_reference_fails_before_provider_effect() {
    let mut value = definition(vec![]);
    value.model.reference = "missing-separator".to_owned();
    let registry = AgentRegistry::new(
        vec![value],
        InMemoryDefinitionStore::default(),
        StaticReferenceCatalog::new(["missing-separator".to_owned()], [], [], []),
    )
    .expect("registry");
    let provider = CountingProvider(Mutex::new(0));
    let tools = FixedToolRegistry::default();
    let memory = InMemoryMemoryStore::default();
    let knowledge = StaticKnowledgeStore::default();
    let sandbox = DenySandbox;
    let runtime =
        LocalAgentRuntime::new(&registry, &provider, &tools, &memory, &knowledge, &sandbox);
    let controls = Controls::new();

    assert_eq!(
        poll_ready(runtime.invoke(
            invocation_context(),
            &AgentId::new("agent").expect("id"),
            String::new(),
            controls.control(),
        )),
        Err(DefinitionError::InvalidReference)
    );
    assert_eq!(*provider.0.lock().expect("calls"), 0);
}

#[test]
fn cancellation_and_deadline_errors_remain_distinguishable() {
    for (gateway, expected) in [
        (LlmError::Cancelled, DefinitionError::Cancelled),
        (
            LlmError::DeadlineExceeded,
            DefinitionError::DeadlineExceeded,
        ),
        (LlmError::LimitExceeded, DefinitionError::LimitExceeded),
    ] {
        let registry = registry(definition(vec![]));
        let provider = StaticProvider::error(gateway);
        let tools = FixedToolRegistry::default();
        let memory = InMemoryMemoryStore::default();
        let knowledge = StaticKnowledgeStore::default();
        let sandbox = DenySandbox;
        let runtime =
            LocalAgentRuntime::new(&registry, &provider, &tools, &memory, &knowledge, &sandbox);
        let controls = Controls::new();
        assert_eq!(
            poll_ready(runtime.invoke(
                invocation_context(),
                &AgentId::new("agent").expect("id"),
                String::new(),
                controls.control(),
            )),
            Err(expected)
        );
    }
}

#[test]
fn registry_validation_and_builtin_protection_are_preserved() {
    let mut invalid = definition(vec![]);
    invalid.instructions.clear();
    assert_eq!(
        validate_definition(&invalid),
        Err(DefinitionError::InvalidDefinition)
    );

    let builtin = definition(vec![]);
    let id = builtin.id.clone();
    let registry = AgentRegistry::new(
        vec![builtin.clone()],
        InMemoryDefinitionStore::default(),
        StaticReferenceCatalog::new(["static.model".to_owned()], [], [], []),
    )
    .expect("registry");
    assert_eq!(
        registry.register(builtin),
        Err(DefinitionError::BuiltinProtected)
    );
    assert_eq!(registry.delete(&id), Err(DefinitionError::BuiltinProtected));
}

#[test]
fn effective_ceiling_remains_canonical_and_non_elevating() {
    let mut value = definition(vec!["allowed"]);
    value.memory = MemoryPolicy {
        enabled: true,
        max_items: 1,
    };
    value.sandbox.allow_execution = true;
    let ceiling = EffectiveCapabilityCeilingV1 {
        allowed_tool_ids: vec![
            "unknown".to_owned(),
            "allowed".to_owned(),
            "allowed".to_owned(),
        ],
        memory_enabled: true,
        knowledge_enabled: true,
        sandbox_execution_allowed: false,
        communication_allowed: true,
    };
    assert_eq!(
        ceiling.intersect(&value).expect("intersection"),
        EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: vec!["allowed".to_owned()],
            memory_enabled: true,
            knowledge_enabled: false,
            sandbox_execution_allowed: false,
            communication_allowed: false,
        }
    );
}

#[test]
fn reserved_name_cannot_be_registered_as_an_ordinary_tool() {
    let value = definition(vec!["factory.memory.recall"]);
    let registry = AgentRegistry::new(
        vec![value],
        InMemoryDefinitionStore::default(),
        StaticReferenceCatalog::new(
            ["static.model".to_owned()],
            [],
            [],
            ["factory.memory.recall".to_owned()],
        ),
    )
    .expect("registry");
    let provider = CountingProvider(Mutex::new(0));
    let tools = FixedToolRegistry::new([("factory.memory.recall".to_owned(), String::new())]);
    let memory = InMemoryMemoryStore::default();
    let knowledge = StaticKnowledgeStore::default();
    let sandbox = DenySandbox;
    let runtime =
        LocalAgentRuntime::new(&registry, &provider, &tools, &memory, &knowledge, &sandbox);
    let controls = Controls::new();
    assert_eq!(
        poll_ready(runtime.invoke(
            invocation_context(),
            &AgentId::new("agent").expect("id"),
            String::new(),
            controls.control(),
        )),
        Err(DefinitionError::InvalidReference)
    );
    assert_eq!(*provider.0.lock().expect("calls"), 0);
}
