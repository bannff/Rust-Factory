use std::{
    future::Future,
    pin::Pin,
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll, Wake, Waker},
    time::Instant,
};

use agent::*;
use llm_gateway::{
    CancellationFuture, CancellationSignal, DeadlineFuture, DeadlineSignal, FinishReason,
    GenerateRequest, GenerateResponse, IdempotencyDisposition, IdempotencyKey, InvocationControl,
    JsonObject, LlmError, LlmProvider, ProviderFuture, ProviderRequestId, TokenUsage,
    ToolCall as GatewayToolCall, ToolName,
};

#[derive(Default)]
struct SwitchingSignal(Arc<AtomicBool>);
impl CancellationSignal for SwitchingSignal {
    fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    fn cancelled(&self) -> CancellationFuture<'_> {
        Box::pin(std::future::pending())
    }
}
impl DeadlineSignal for SwitchingSignal {
    fn instant(&self) -> Instant {
        Instant::now()
    }
    fn is_elapsed(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
    fn elapsed(&self) -> DeadlineFuture<'_> {
        Box::pin(std::future::pending())
    }
}

struct Controls {
    key: IdempotencyKey,
    cancellation: SwitchingSignal,
    deadline: SwitchingSignal,
}
impl Controls {
    fn new(key: &str) -> Self {
        Self {
            key: IdempotencyKey::new(key).expect("valid key"),
            cancellation: SwitchingSignal::default(),
            deadline: SwitchingSignal::default(),
        }
    }
    fn control(&self) -> InvocationControl<'_> {
        InvocationControl {
            idempotency_key: &self.key,
            cancellation: &self.cancellation,
            deadline: &self.deadline,
        }
    }
    fn cancellation_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancellation.0)
    }
    fn deadline_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.deadline.0)
    }
}

struct NoopWake;
impl Wake for NoopWake {
    fn wake(self: Arc<Self>) {}
}
fn poll_once<T>(future: Pin<&mut (dyn Future<Output = T> + Send)>) -> Poll<T> {
    let waker = Waker::from(Arc::new(NoopWake));
    Future::poll(future, &mut Context::from_waker(&waker))
}
fn ready<T>(future: impl Future<Output = T> + Send) -> T {
    let mut future = Box::pin(future);
    match poll_once(future.as_mut()) {
        Poll::Ready(value) => value,
        Poll::Pending => panic!("deterministic future unexpectedly pending"),
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct RequestSnapshot {
    provider: String,
    model: String,
    system: Option<String>,
    input: String,
    tools: Vec<(String, String)>,
    max_output_tokens: u32,
    key: String,
    cancellation: usize,
    deadline: usize,
}

#[derive(Clone)]
struct ResponsePlan {
    text: String,
    calls: Vec<(String, String)>,
    request_id: Option<String>,
    finish: FinishReason,
    usage: Option<TokenUsage>,
    idempotency: IdempotencyDisposition,
    set_after_response: Vec<Arc<AtomicBool>>,
}
impl Default for ResponsePlan {
    fn default() -> Self {
        Self {
            text: "ok".to_owned(),
            calls: vec![],
            request_id: None,
            finish: FinishReason::Stop,
            usage: None,
            idempotency: IdempotencyDisposition::Unsupported,
            set_after_response: vec![],
        }
    }
}

enum ProviderPlan {
    Response(ResponsePlan),
    Error(LlmError),
    Pending(Arc<AtomicBool>),
}
struct RecordingProvider {
    plan: ProviderPlan,
    requests: Arc<Mutex<Vec<RequestSnapshot>>>,
}
impl RecordingProvider {
    fn response(plan: ResponsePlan) -> (Self, Arc<Mutex<Vec<RequestSnapshot>>>) {
        let requests = Arc::new(Mutex::new(vec![]));
        (
            Self {
                plan: ProviderPlan::Response(plan),
                requests: Arc::clone(&requests),
            },
            requests,
        )
    }
    fn error(error: LlmError) -> Self {
        Self {
            plan: ProviderPlan::Error(error),
            requests: Arc::new(Mutex::new(vec![])),
        }
    }
}
impl LlmProvider for RecordingProvider {
    fn generate<'a>(
        &'a self,
        request: &'a GenerateRequest,
        control: InvocationControl<'a>,
    ) -> ProviderFuture<'a> {
        self.requests
            .lock()
            .expect("requests")
            .push(RequestSnapshot {
                provider: request.provider_id().as_str().to_owned(),
                model: request.model_id().as_str().to_owned(),
                system: request.prompt().system().map(str::to_owned),
                input: request.prompt().input().to_owned(),
                tools: request
                    .tools()
                    .iter()
                    .map(|tool| {
                        (
                            tool.name().as_str().to_owned(),
                            tool.input_schema().canonical().to_owned(),
                        )
                    })
                    .collect(),
                max_output_tokens: request.limits().max_output_tokens(),
                key: control.idempotency_key.as_str().to_owned(),
                cancellation: std::ptr::from_ref(control.cancellation).cast::<()>() as usize,
                deadline: std::ptr::from_ref(control.deadline).cast::<()>() as usize,
            });
        match &self.plan {
            ProviderPlan::Response(plan) => {
                let calls = plan
                    .calls
                    .iter()
                    .map(|(name, arguments)| {
                        GatewayToolCall::new(
                            ToolName::new(name.clone()).expect("tool name"),
                            JsonObject::new(arguments).expect("object arguments"),
                        )
                        .expect("tool call")
                    })
                    .collect::<Vec<_>>();
                let result = GenerateResponse::new(
                    request,
                    plan.text.clone(),
                    calls,
                    plan.request_id
                        .as_ref()
                        .map(|id| ProviderRequestId::new(id.clone()).expect("request id")),
                    plan.finish,
                    plan.usage,
                    plan.idempotency,
                );
                let set_after_response = plan.set_after_response.clone();
                Box::pin(async move {
                    for signal in set_after_response {
                        signal.store(true, Ordering::SeqCst);
                    }
                    result
                })
            }
            ProviderPlan::Error(error) => {
                let error = *error;
                Box::pin(async move { Err(error) })
            }
            ProviderPlan::Pending(dropped) => Box::pin(PendingProviderFuture {
                dropped: Arc::clone(dropped),
            }),
        }
    }
}
struct PendingProviderFuture {
    dropped: Arc<AtomicBool>,
}
impl Future for PendingProviderFuture {
    type Output = Result<GenerateResponse, LlmError>;
    fn poll(self: Pin<&mut Self>, _: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}
impl Drop for PendingProviderFuture {
    fn drop(&mut self) {
        self.dropped.store(true, Ordering::SeqCst);
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

fn definition(tools: &[&str]) -> AgentDefinitionV1 {
    AgentDefinitionV1 {
        version: DefinitionVersion::V1,
        id: AgentId::new("agent").expect("id"),
        name: "Agent".to_owned(),
        description: "Adversarial fixture".to_owned(),
        model: ModelPolicy {
            reference: "provider.model".to_owned(),
        },
        instructions: "System instructions".to_owned(),
        skills: vec![],
        steering: vec![],
        allowed_tool_ids: tools.iter().map(|value| (*value).to_owned()).collect(),
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
            max_tool_calls: 64,
            max_output_bytes: 65_536,
        },
    }
}
fn registry(
    value: AgentDefinitionV1,
) -> AgentRegistry<InMemoryDefinitionStore, StaticReferenceCatalog> {
    let tools = value.allowed_tool_ids.clone();
    let model = value.model.reference.clone();
    AgentRegistry::new(
        vec![value],
        InMemoryDefinitionStore::default(),
        StaticReferenceCatalog::new([model], [], [], tools),
    )
    .expect("registry")
}

#[derive(Default)]
struct EffectCounts {
    tool_resolves: AtomicUsize,
    tool_invokes: AtomicUsize,
    memory_recalls: AtomicUsize,
    memory_writes: AtomicUsize,
    knowledge_searches: AtomicUsize,
    sandbox_executes: AtomicUsize,
    contexts: Mutex<Vec<InvocationContextV1>>,
    cancel_after_tool_invoke: Mutex<Option<Arc<AtomicBool>>>,
}
struct RecordingTools {
    counts: Arc<EffectCounts>,
    output: String,
}
impl ToolRegistry for RecordingTools {
    fn resolve(&self, id: &str) -> Result<ToolDescriptor, DefinitionError> {
        self.counts.tool_resolves.fetch_add(1, Ordering::SeqCst);
        Ok(ToolDescriptor { id: id.to_owned() })
    }
    fn invoke(&self, _: &ToolDescriptor, request: ToolRequest) -> Result<String, DefinitionError> {
        self.counts.tool_invokes.fetch_add(1, Ordering::SeqCst);
        self.counts
            .contexts
            .lock()
            .expect("contexts")
            .push(request.context);
        if let Some(flag) = self
            .counts
            .cancel_after_tool_invoke
            .lock()
            .expect("cancel switch")
            .take()
        {
            flag.store(true, Ordering::SeqCst);
        }
        Ok(self.output.clone())
    }
}
struct RecordingMemory(Arc<EffectCounts>);
impl MemoryStore for RecordingMemory {
    fn recall(&self, request: MemoryRequest) -> Result<Vec<String>, DefinitionError> {
        self.0.memory_recalls.fetch_add(1, Ordering::SeqCst);
        self.0
            .contexts
            .lock()
            .expect("contexts")
            .push(request.context);
        Ok(vec![])
    }
    fn write(&self, request: MemoryRequest, _: String) -> Result<(), DefinitionError> {
        self.0.memory_writes.fetch_add(1, Ordering::SeqCst);
        self.0
            .contexts
            .lock()
            .expect("contexts")
            .push(request.context);
        Ok(())
    }
}
struct RecordingKnowledge(Arc<EffectCounts>);
impl KnowledgeStore for RecordingKnowledge {
    fn search(&self, request: KnowledgeRequest) -> Result<Vec<String>, DefinitionError> {
        self.0.knowledge_searches.fetch_add(1, Ordering::SeqCst);
        self.0
            .contexts
            .lock()
            .expect("contexts")
            .push(request.context);
        Ok(vec![])
    }
}
struct RecordingSandbox(Arc<EffectCounts>);
impl Sandbox for RecordingSandbox {
    fn execute(&self, request: SandboxRequest) -> Result<String, DefinitionError> {
        self.0.sandbox_executes.fetch_add(1, Ordering::SeqCst);
        self.0
            .contexts
            .lock()
            .expect("contexts")
            .push(request.context);
        Ok(String::new())
    }
}

fn invoke_contextual(
    context: InvocationContextV1,
    value: AgentDefinitionV1,
    provider: &RecordingProvider,
    controls: &Controls,
    counts: Arc<EffectCounts>,
    tool_output: String,
) -> Result<InvocationResult, DefinitionError> {
    let registry = registry(value);
    let tools = RecordingTools {
        counts: Arc::clone(&counts),
        output: tool_output,
    };
    let memory = RecordingMemory(Arc::clone(&counts));
    let knowledge = RecordingKnowledge(Arc::clone(&counts));
    let sandbox = RecordingSandbox(counts);
    let runtime =
        LocalAgentRuntime::new(&registry, provider, &tools, &memory, &knowledge, &sandbox);
    ready(runtime.invoke(
        context,
        &AgentId::new("agent").expect("id"),
        "input".to_owned(),
        controls.control(),
    ))
}

fn invoke(
    value: AgentDefinitionV1,
    provider: &RecordingProvider,
    controls: &Controls,
    counts: Arc<EffectCounts>,
    tool_output: String,
) -> Result<InvocationResult, DefinitionError> {
    invoke_contextual(
        invocation_context(),
        value,
        provider,
        controls,
        counts,
        tool_output,
    )
}

fn effect_count(counts: &EffectCounts) -> usize {
    counts.tool_invokes.load(Ordering::SeqCst)
        + counts.memory_recalls.load(Ordering::SeqCst)
        + counts.memory_writes.load(Ordering::SeqCst)
        + counts.knowledge_searches.load(Ordering::SeqCst)
        + counts.sandbox_executes.load(Ordering::SeqCst)
}

#[test]
fn every_returned_call_is_planned_before_the_first_effect() {
    let cases = [
        ("malformed", "allowed", r"{}".to_owned(), false),
        ("unknown", "unknown", r#"{"input":"x"}"#.to_owned(), false),
        (
            "denied",
            "factory.memory.recall",
            r#"{"query":"x"}"#.to_owned(),
            false,
        ),
        (
            "oversized",
            "factory.sandbox.execute",
            format!(
                r#"{{"action":"x","arguments":["{}"]}}"#,
                "x".repeat(MAX_SANDBOX_ARGUMENT_BYTES + 1)
            ),
            true,
        ),
        (
            "invalid",
            "allowed",
            r#"{"input":"x","extra":true}"#.to_owned(),
            false,
        ),
    ];
    for (case, bad_name, bad_arguments, enable_sandbox) in cases {
        let mut value = definition(&["allowed"]);
        value.sandbox.allow_execution = enable_sandbox;
        let (provider, _) = RecordingProvider::response(ResponsePlan {
            calls: vec![
                ("allowed".to_owned(), r#"{"input":"first"}"#.to_owned()),
                (bad_name.to_owned(), bad_arguments),
            ],
            ..ResponsePlan::default()
        });
        let counts = Arc::new(EffectCounts::default());
        let result = invoke(
            value,
            &provider,
            &Controls::new("key"),
            Arc::clone(&counts),
            String::new(),
        );
        assert!(result.is_err(), "{case} later call unexpectedly succeeded");
        assert_eq!(
            effect_count(&counts),
            0,
            "{case} later call leaked an effect"
        );
    }
}

#[test]
fn provider_control_switches_block_the_first_effect_with_cancellation_priority() {
    for (cancel, elapsed, expected) in [
        (true, false, DefinitionError::Cancelled),
        (false, true, DefinitionError::DeadlineExceeded),
        (true, true, DefinitionError::Cancelled),
    ] {
        let controls = Controls::new("key");
        let mut switches = vec![];
        if cancel {
            switches.push(controls.cancellation_flag());
        }
        if elapsed {
            switches.push(controls.deadline_flag());
        }
        let (provider, _) = RecordingProvider::response(ResponsePlan {
            calls: vec![("allowed".to_owned(), r#"{"input":"x"}"#.to_owned())],
            set_after_response: switches,
            ..ResponsePlan::default()
        });
        let counts = Arc::new(EffectCounts::default());
        assert_eq!(
            invoke(
                definition(&["allowed"]),
                &provider,
                &controls,
                Arc::clone(&counts),
                String::new(),
            ),
            Err(expected)
        );
        assert_eq!(effect_count(&counts), 0);
    }
}

#[test]
fn control_switch_between_effects_blocks_the_next_effect() {
    let controls = Controls::new("key");
    let (provider, _) = RecordingProvider::response(ResponsePlan {
        calls: vec![
            ("allowed".to_owned(), r#"{"input":"first"}"#.to_owned()),
            ("allowed".to_owned(), r#"{"input":"second"}"#.to_owned()),
        ],
        ..ResponsePlan::default()
    });
    let counts = Arc::new(EffectCounts::default());
    *counts
        .cancel_after_tool_invoke
        .lock()
        .expect("cancel switch") = Some(controls.cancellation_flag());
    assert_eq!(
        invoke(
            definition(&["allowed"]),
            &provider,
            &controls,
            Arc::clone(&counts),
            String::new(),
        ),
        Err(DefinitionError::Cancelled)
    );
    assert_eq!(counts.tool_invokes.load(Ordering::SeqCst), 1);
    assert_eq!(effect_count(&counts), 1);
}

#[test]
fn gateway_request_contains_only_normalized_model_prompt_limits_and_effective_tools() {
    let mut value = definition(&["allowed", "denied"]);
    value.memory = MemoryPolicy {
        enabled: true,
        max_items: 2,
    };
    value.knowledge = KnowledgePolicy {
        enabled: true,
        max_results: 3,
    };
    value.sandbox.allow_execution = true;
    value.limits.max_output_bytes = 321;
    let registry = registry(value);
    let (provider, requests) = RecordingProvider::response(ResponsePlan::default());
    let counts = Arc::new(EffectCounts::default());
    let tools = RecordingTools {
        counts: Arc::clone(&counts),
        output: String::new(),
    };
    let memory = RecordingMemory(Arc::clone(&counts));
    let knowledge = RecordingKnowledge(Arc::clone(&counts));
    let sandbox = RecordingSandbox(counts);
    let runtime =
        LocalAgentRuntime::new(&registry, &provider, &tools, &memory, &knowledge, &sandbox);
    let ceiling = EffectiveCapabilityCeilingV1 {
        allowed_tool_ids: vec!["allowed".to_owned()],
        memory_enabled: true,
        knowledge_enabled: false,
        sandbox_execution_allowed: true,
        communication_allowed: false,
    };
    let controls = Controls::new("stable-key");
    ready(runtime.invoke_with_ceiling(
        invocation_context(),
        &AgentId::new("agent").expect("id"),
        "user input".to_owned(),
        &ceiling,
        controls.control(),
    ))
    .expect("invoke");

    let requests = requests.lock().expect("requests");
    assert_eq!(requests.len(), 1);
    let request = &requests[0];
    assert_eq!(request.provider, "provider");
    assert_eq!(request.model, "model");
    assert_eq!(request.system.as_deref(), Some("System instructions"));
    assert_eq!(request.input, "user input");
    assert_eq!(request.max_output_tokens, 321);
    assert_eq!(request.key, "stable-key");
    assert_eq!(
        request.cancellation,
        std::ptr::from_ref(&controls.cancellation).cast::<()>() as usize
    );
    assert_eq!(
        request.deadline,
        std::ptr::from_ref(&controls.deadline).cast::<()>() as usize
    );
    assert_eq!(
        request
            .tools
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        [
            "allowed",
            "factory.memory.recall",
            "factory.memory.write",
            "factory.sandbox.execute",
        ]
    );
    let serialized = format!("{request:?}");
    for forbidden in [
        "agent_id",
        "capability_scope",
        "policy",
        "grant",
        "tenant",
        "principal",
        "credential",
        "endpoint",
    ] {
        assert!(
            !serialized.contains(forbidden),
            "gateway request leaked {forbidden}"
        );
    }
}

#[test]
fn every_reserved_tool_has_the_exact_closed_agent_owned_schema() {
    let mut value = definition(&[]);
    value.memory = MemoryPolicy {
        enabled: true,
        max_items: 1,
    };
    value.knowledge = KnowledgePolicy {
        enabled: true,
        max_results: 1,
    };
    value.sandbox.allow_execution = true;
    let (provider, requests) = RecordingProvider::response(ResponsePlan::default());
    invoke(
        value,
        &provider,
        &Controls::new("key"),
        Arc::new(EffectCounts::default()),
        String::new(),
    )
    .expect("invoke");
    let tools = &requests.lock().expect("requests")[0].tools;
    assert_eq!(tools, &vec![
        ("factory.memory.recall".to_owned(), r#"{"additionalProperties":false,"properties":{"query":{"type":"string"}},"required":["query"],"type":"object"}"#.to_owned()),
        ("factory.memory.write".to_owned(), r#"{"additionalProperties":false,"properties":{"value":{"type":"string"}},"required":["value"],"type":"object"}"#.to_owned()),
        ("factory.knowledge.search".to_owned(), r#"{"additionalProperties":false,"properties":{"query":{"type":"string"}},"required":["query"],"type":"object"}"#.to_owned()),
        ("factory.sandbox.execute".to_owned(), r#"{"additionalProperties":false,"properties":{"action":{"type":"string"},"arguments":{"items":{"type":"string"},"type":"array"}},"required":["action","arguments"],"type":"object"}"#.to_owned()),
    ]);
}

#[test]
fn malformed_reserved_and_ordinary_arguments_never_reach_effect_ports() {
    let cases = [
        ("allowed", r"{}"),
        ("allowed", r#"{"input":1}"#),
        ("allowed", r#"{"input":"x","extra":true}"#),
        ("factory.memory.recall", r"{}"),
        ("factory.memory.recall", r#"{"query":false}"#),
        ("factory.memory.recall", r#"{"query":"x","extra":0}"#),
        ("factory.memory.write", r"{}"),
        ("factory.memory.write", r#"{"value":[]}"#),
        ("factory.memory.write", r#"{"value":"x","extra":0}"#),
        ("factory.knowledge.search", r"{}"),
        ("factory.knowledge.search", r#"{"query":null}"#),
        ("factory.knowledge.search", r#"{"query":"x","extra":0}"#),
        ("factory.sandbox.execute", r"{}"),
        ("factory.sandbox.execute", r#"{"action":1,"arguments":[]}"#),
        (
            "factory.sandbox.execute",
            r#"{"action":"x","arguments":[1]}"#,
        ),
        (
            "factory.sandbox.execute",
            r#"{"action":"x","arguments":[],"extra":0}"#,
        ),
    ];
    for (name, arguments) in cases {
        let mut value = definition(&["allowed"]);
        value.memory = MemoryPolicy {
            enabled: true,
            max_items: 1,
        };
        value.knowledge = KnowledgePolicy {
            enabled: true,
            max_results: 1,
        };
        value.sandbox.allow_execution = true;
        let (provider, _) = RecordingProvider::response(ResponsePlan {
            calls: vec![(name.to_owned(), arguments.to_owned())],
            ..ResponsePlan::default()
        });
        let counts = Arc::new(EffectCounts::default());
        let result = invoke(
            value,
            &provider,
            &Controls::new("key"),
            Arc::clone(&counts),
            String::new(),
        );
        assert_eq!(
            result,
            Err(DefinitionError::InvalidDefinition),
            "case {name} {arguments}"
        );
        assert_eq!(counts.tool_invokes.load(Ordering::SeqCst), 0);
        assert_eq!(counts.memory_recalls.load(Ordering::SeqCst), 0);
        assert_eq!(counts.memory_writes.load(Ordering::SeqCst), 0);
        assert_eq!(counts.knowledge_searches.load(Ordering::SeqCst), 0);
        assert_eq!(counts.sandbox_executes.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn reserved_limits_are_checked_before_effects() {
    // String-valued reserved arguments cannot exceed Agent's 16 KiB value
    // limit while satisfying the gateway's stricter 16 KiB object limit.
    // These two Agent limits remain independently reachable.
    let cases = [
        (
            "factory.sandbox.execute",
            format!(
                r#"{{"action":"x","arguments":[{}]}}"#,
                (0..=MAX_SANDBOX_ARGUMENTS)
                    .map(|_| "\"x\"")
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        ),
        (
            "factory.sandbox.execute",
            format!(
                r#"{{"action":"x","arguments":["{}"]}}"#,
                "x".repeat(MAX_SANDBOX_ARGUMENT_BYTES + 1)
            ),
        ),
    ];
    for (name, arguments) in cases {
        let mut value = definition(&[]);
        value.memory = MemoryPolicy {
            enabled: true,
            max_items: 1,
        };
        value.knowledge = KnowledgePolicy {
            enabled: true,
            max_results: 1,
        };
        value.sandbox.allow_execution = true;
        let (provider, _) = RecordingProvider::response(ResponsePlan {
            calls: vec![(name.to_owned(), arguments)],
            ..ResponsePlan::default()
        });
        let counts = Arc::new(EffectCounts::default());
        let result = invoke(
            value,
            &provider,
            &Controls::new("key"),
            Arc::clone(&counts),
            String::new(),
        );
        assert!(
            matches!(
                result,
                Err(DefinitionError::LimitExceeded | DefinitionError::AdapterFailure)
            ),
            "{name}: {result:?}"
        );
        assert_eq!(
            counts.memory_recalls.load(Ordering::SeqCst)
                + counts.memory_writes.load(Ordering::SeqCst)
                + counts.knowledge_searches.load(Ordering::SeqCst)
                + counts.sandbox_executes.load(Ordering::SeqCst),
            0
        );
    }
}

#[test]
fn denied_capabilities_are_absent_from_request_and_cannot_reach_effects() {
    let mut value = definition(&["allowed"]);
    value.memory = MemoryPolicy {
        enabled: true,
        max_items: 1,
    };
    value.knowledge = KnowledgePolicy {
        enabled: true,
        max_results: 1,
    };
    value.sandbox.allow_execution = true;
    let registry = registry(value);
    let (provider, requests) = RecordingProvider::response(ResponsePlan::default());
    let counts = Arc::new(EffectCounts::default());
    let tools = RecordingTools {
        counts: Arc::clone(&counts),
        output: String::new(),
    };
    let memory = RecordingMemory(Arc::clone(&counts));
    let knowledge = RecordingKnowledge(Arc::clone(&counts));
    let sandbox = RecordingSandbox(Arc::clone(&counts));
    let runtime =
        LocalAgentRuntime::new(&registry, &provider, &tools, &memory, &knowledge, &sandbox);
    let denied = EffectiveCapabilityCeilingV1 {
        allowed_tool_ids: vec![],
        memory_enabled: false,
        knowledge_enabled: false,
        sandbox_execution_allowed: false,
        communication_allowed: false,
    };
    ready(runtime.invoke_with_ceiling(
        invocation_context(),
        &AgentId::new("agent").expect("id"),
        String::new(),
        &denied,
        Controls::new("key").control(),
    ))
    .expect("invoke");
    assert!(requests.lock().expect("requests")[0].tools.is_empty());
    assert_eq!(counts.tool_resolves.load(Ordering::SeqCst), 0);
    assert_eq!(counts.tool_invokes.load(Ordering::SeqCst), 0);
}

#[test]
fn response_and_tool_output_share_one_checked_output_budget() {
    for (tool_output, expected) in [
        ("x".repeat(5), Ok(())),
        ("x".repeat(6), Err(DefinitionError::LimitExceeded)),
    ] {
        let mut value = definition(&["allowed"]);
        value.limits.max_output_bytes = 10;
        let (provider, _) = RecordingProvider::response(ResponsePlan {
            text: "12345".to_owned(),
            calls: vec![("allowed".to_owned(), r#"{"input":"x"}"#.to_owned())],
            ..ResponsePlan::default()
        });
        let result = invoke(
            value,
            &provider,
            &Controls::new("key"),
            Arc::new(EffectCounts::default()),
            tool_output,
        )
        .map(|_| ());
        assert_eq!(result, expected);
    }
}

#[test]
fn gateway_errors_map_to_the_closed_agent_taxonomy_without_details() {
    let cases = [
        (LlmError::InvalidRequest, DefinitionError::InvalidDefinition),
        (LlmError::LimitExceeded, DefinitionError::LimitExceeded),
        (LlmError::Cancelled, DefinitionError::Cancelled),
        (
            LlmError::DeadlineExceeded,
            DefinitionError::DeadlineExceeded,
        ),
        (LlmError::Unsupported, DefinitionError::AdapterFailure),
        (LlmError::Authentication, DefinitionError::AdapterFailure),
        (LlmError::RateLimited, DefinitionError::AdapterFailure),
        (LlmError::Unavailable, DefinitionError::AdapterFailure),
        (LlmError::ProviderRejected, DefinitionError::AdapterFailure),
        (LlmError::ProtocolViolation, DefinitionError::AdapterFailure),
    ];
    for (gateway, agent_error) in cases {
        let result = invoke(
            definition(&[]),
            &RecordingProvider::error(gateway),
            &Controls::new("key"),
            Arc::new(EffectCounts::default()),
            String::new(),
        );
        assert_eq!(result, Err(agent_error));
    }
    assert_eq!(
        DefinitionError::AdapterFailure.to_string(),
        "agent definition operation failed: AdapterFailure"
    );
}

#[test]
fn model_evidence_is_an_exact_safe_projection() {
    let (provider, _) = RecordingProvider::response(ResponsePlan {
        text: "safe".to_owned(),
        request_id: Some("request-7".to_owned()),
        finish: FinishReason::ContentFilter,
        usage: Some(TokenUsage::new(11, 7, Some(18)).expect("usage")),
        idempotency: IdempotencyDisposition::Accepted,
        ..ResponsePlan::default()
    });
    let result = invoke(
        definition(&[]),
        &provider,
        &Controls::new("secret-looking-key"),
        Arc::new(EffectCounts::default()),
        String::new(),
    )
    .expect("invoke");
    assert_eq!(
        result.model_evidence,
        InvocationModelEvidence {
            provider_id: "provider".to_owned(),
            model_id: "model".to_owned(),
            provider_request_id: Some("request-7".to_owned()),
            finish_reason: InvocationModelFinishReason::ContentFilter,
            token_usage: Some(InvocationModelTokenUsage {
                input_tokens: 11,
                output_tokens: 7,
                total_tokens: 18
            }),
            idempotency: InvocationModelIdempotency::Accepted,
        }
    );
    let debug = format!("{:?}", result.model_evidence);
    for forbidden in [
        "System instructions",
        "secret-looking-key",
        "credential",
        "endpoint",
        "headers",
    ] {
        assert!(!debug.contains(forbidden));
    }
}

#[test]
fn dropping_unpolled_and_pending_invocations_has_no_detached_work() {
    let provider_calls = Arc::new(Mutex::new(vec![]));
    let dropped = Arc::new(AtomicBool::new(false));
    let provider = RecordingProvider {
        plan: ProviderPlan::Pending(Arc::clone(&dropped)),
        requests: Arc::clone(&provider_calls),
    };
    let registry = registry(definition(&[]));
    let counts = Arc::new(EffectCounts::default());
    let tools = RecordingTools {
        counts: Arc::clone(&counts),
        output: String::new(),
    };
    let memory = RecordingMemory(Arc::clone(&counts));
    let knowledge = RecordingKnowledge(Arc::clone(&counts));
    let sandbox = RecordingSandbox(counts);
    let runtime =
        LocalAgentRuntime::new(&registry, &provider, &tools, &memory, &knowledge, &sandbox);
    let controls = Controls::new("key");
    let id = AgentId::new("agent").expect("id");

    drop(runtime.invoke(invocation_context(), &id, String::new(), controls.control()));
    assert!(
        provider_calls.lock().expect("calls").is_empty(),
        "unpolled invocation performed work"
    );

    let mut future = runtime.invoke(invocation_context(), &id, String::new(), controls.control());
    assert!(poll_once(future.as_mut()).is_pending());
    assert_eq!(provider_calls.lock().expect("calls").len(), 1);
    drop(future);
    assert!(
        dropped.load(Ordering::SeqCst),
        "dropping Agent future did not drop provider future"
    );
}

struct CountingStore {
    value: AgentDefinitionV1,
    loads: Arc<AtomicUsize>,
}
impl DefinitionStore for CountingStore {
    fn load(&self, _: &AgentId) -> Result<Option<AgentDefinitionV1>, DefinitionError> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        Ok(Some(self.value.clone()))
    }
    fn list(&self) -> Result<Vec<AgentDefinitionV1>, DefinitionError> {
        Ok(vec![self.value.clone()])
    }
    fn save(&self, _: AgentDefinitionV1) -> Result<(), DefinitionError> {
        Ok(())
    }
    fn delete(&self, _: &AgentId) -> Result<(), DefinitionError> {
        Ok(())
    }
}

#[test]
fn each_invocation_loads_exactly_one_definition_snapshot() {
    let value = definition(&[]);
    let loads = Arc::new(AtomicUsize::new(0));
    let registry = AgentRegistry::new(
        vec![],
        CountingStore {
            value,
            loads: Arc::clone(&loads),
        },
        StaticReferenceCatalog::new(["provider.model".to_owned()], [], [], []),
    )
    .expect("registry");
    let (provider, _) = RecordingProvider::response(ResponsePlan::default());
    let counts = Arc::new(EffectCounts::default());
    let tools = RecordingTools {
        counts: Arc::clone(&counts),
        output: String::new(),
    };
    let memory = RecordingMemory(Arc::clone(&counts));
    let knowledge = RecordingKnowledge(Arc::clone(&counts));
    let sandbox = RecordingSandbox(counts);
    let runtime =
        LocalAgentRuntime::new(&registry, &provider, &tools, &memory, &knowledge, &sandbox);
    let controls = Controls::new("key");
    ready(runtime.invoke(
        invocation_context(),
        &AgentId::new("agent").expect("id"),
        String::new(),
        controls.control(),
    ))
    .expect("invoke");
    assert_eq!(loads.load(Ordering::SeqCst), 1);
}

#[test]
fn malformed_model_reference_variants_fail_before_provider_or_tool_effects() {
    for reference in ["provider", "provider.model.extra"] {
        let mut value = definition(&[]);
        value.model.reference = reference.to_owned();
        let registry = AgentRegistry::new(
            vec![value],
            InMemoryDefinitionStore::default(),
            StaticReferenceCatalog::new([reference.to_owned()], [], [], []),
        )
        .expect("registry");
        let (provider, requests) = RecordingProvider::response(ResponsePlan::default());
        let counts = Arc::new(EffectCounts::default());
        let tools = RecordingTools {
            counts: Arc::clone(&counts),
            output: String::new(),
        };
        let memory = RecordingMemory(Arc::clone(&counts));
        let knowledge = RecordingKnowledge(Arc::clone(&counts));
        let sandbox = RecordingSandbox(counts);
        let runtime =
            LocalAgentRuntime::new(&registry, &provider, &tools, &memory, &knowledge, &sandbox);
        assert_eq!(
            ready(runtime.invoke(
                invocation_context(),
                &AgentId::new("agent").expect("id"),
                String::new(),
                Controls::new("key").control()
            )),
            Err(DefinitionError::InvalidReference)
        );
        assert!(requests.lock().expect("requests").is_empty());
    }
}

proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config { cases: 64, failure_persistence: None, ..proptest::test_runner::Config::default() })]
    #[test]
    fn ceiling_intersection_is_order_independent_and_non_elevating(
        definition_tools in proptest::collection::vec(0_u8..8, 0..16),
        ceiling_tools in proptest::collection::vec(0_u8..8, 0..16),
    ) {
        let mut value = definition(&[]);
        value.allowed_tool_ids = definition_tools.iter().map(|index| format!("tool{index}")).collect();
        let ceiling = EffectiveCapabilityCeilingV1 {
            allowed_tool_ids: ceiling_tools.iter().map(|index| format!("tool{index}")).collect(),
            memory_enabled: false, knowledge_enabled: false, sandbox_execution_allowed: false, communication_allowed: false,
        };
        let intersection = ceiling.intersect(&value).expect("intersection");
        let expected = definition_tools.iter().filter(|index| ceiling_tools.contains(index)).map(|index| format!("tool{index}")).collect::<std::collections::BTreeSet<_>>().into_iter().collect::<Vec<_>>();
        proptest::prop_assert_eq!(&intersection.allowed_tool_ids, &expected);
        proptest::prop_assert!(intersection.allowed_tool_ids.iter().all(|tool| value.allowed_tool_ids.contains(tool) && ceiling.allowed_tool_ids.contains(tool)));
        let mut reordered_value = value.clone();
        reordered_value.allowed_tool_ids.reverse();
        let mut reordered_ceiling = ceiling.clone();
        reordered_ceiling.allowed_tool_ids.reverse();
        proptest::prop_assert_eq!(reordered_ceiling.intersect(&reordered_value).expect("reordered"), intersection);
    }
}

#[test]
fn public_identifier_definition_and_reference_contracts_remain_active() {
    for invalid in ["", "Upper", "has space", "has.dot", &"x".repeat(129)] {
        assert_eq!(AgentId::new(invalid), Err(DefinitionError::InvalidId));
    }
    let mut value = definition(&[]);
    value.instructions.clear();
    assert_eq!(
        validate_definition(&value),
        Err(DefinitionError::InvalidDefinition)
    );
    for invalid in [
        "/tmp/model",
        "provider:model",
        "../model",
        "UPPER",
        "trailing-",
    ] {
        let mut value = definition(&[]);
        value.model.reference = invalid.to_owned();
        assert_eq!(
            validate_definition(&value),
            Err(DefinitionError::InvalidReference)
        );
    }
    let mut value = definition(&[]);
    value.limits.max_tool_calls = MAX_TOOL_CALLS + 1;
    assert_eq!(
        validate_definition(&value),
        Err(DefinitionError::InvalidDefinition)
    );
}

#[test]
fn scope_digest_is_canonical_and_covers_policy_fields() {
    let mut left = definition(&["b", "a", "a"]);
    left.skills = vec!["b".to_owned(), "a".to_owned()];
    left.steering = vec!["y".to_owned(), "x".to_owned()];
    let mut right = left.clone();
    right.allowed_tool_ids.reverse();
    right.skills.reverse();
    right.steering.reverse();
    let tools = [
        ToolDescriptor { id: "a".to_owned() },
        ToolDescriptor { id: "b".to_owned() },
    ];
    let digest = capability_scope_digest(&left, &tools);
    assert_eq!(digest.len(), 64);
    assert_eq!(digest, capability_scope_digest(&right, &tools));

    let mut changed = left.clone();
    changed.communication.allow_messages = true;
    assert_ne!(digest, capability_scope_digest(&changed, &tools));
    changed = left.clone();
    changed.instructions.push('!');
    assert_ne!(digest, capability_scope_digest(&changed, &tools));
    assert_ne!(digest, capability_scope_digest(&left, &tools[..1]));
}

#[test]
fn duplicate_ordinary_tool_references_canonicalize_to_one_gateway_name() {
    let (provider, requests) = RecordingProvider::response(ResponsePlan::default());
    invoke(
        definition(&["allowed", "allowed"]),
        &provider,
        &Controls::new("key"),
        Arc::new(EffectCounts::default()),
        String::new(),
    )
    .expect("invoke");
    assert_eq!(
        requests.lock().expect("requests")[0]
            .tools
            .iter()
            .map(|(name, _)| name.as_str())
            .collect::<Vec<_>>(),
        ["allowed"]
    );
}

#[test]
fn undeclared_ordinary_or_reserved_calls_fail_before_agent_effects() {
    for undeclared in [
        "unknown",
        "factory.memory.recall",
        "factory.memory.write",
        "factory.knowledge.search",
        "factory.sandbox.execute",
    ] {
        let arguments = match undeclared {
            "factory.memory.recall" | "factory.knowledge.search" => r#"{"query":"x"}"#,
            "factory.memory.write" => r#"{"value":"x"}"#,
            "factory.sandbox.execute" => r#"{"action":"x","arguments":[]}"#,
            _ => r#"{"input":"x"}"#,
        };
        let (provider, _) = RecordingProvider::response(ResponsePlan {
            calls: vec![(undeclared.to_owned(), arguments.to_owned())],
            ..ResponsePlan::default()
        });
        let counts = Arc::new(EffectCounts::default());
        let result = invoke(
            definition(&[]),
            &provider,
            &Controls::new("key"),
            Arc::clone(&counts),
            String::new(),
        );
        assert_eq!(result, Err(DefinitionError::AdapterFailure), "{undeclared}");
        assert_eq!(counts.tool_invokes.load(Ordering::SeqCst), 0);
        assert_eq!(counts.memory_recalls.load(Ordering::SeqCst), 0);
        assert_eq!(counts.memory_writes.load(Ordering::SeqCst), 0);
        assert_eq!(counts.knowledge_searches.load(Ordering::SeqCst), 0);
        assert_eq!(counts.sandbox_executes.load(Ordering::SeqCst), 0);
    }
}

#[test]
fn invocation_id_grammar_has_exact_boundaries_for_all_distinct_types() {
    macro_rules! assert_id_contract {
        ($type:ty) => {{
            assert_eq!(<$type>::new("a").expect("minimum").as_str(), "a");
            let maximum = format!("a{}", "z_-0".repeat(31) + "zzz");
            assert_eq!(maximum.len(), 128);
            assert_eq!(
                <$type>::new(maximum.clone()).expect("maximum").as_str(),
                maximum
            );
            for invalid in [
                String::new(),
                "Upper".to_owned(),
                "-leading".to_owned(),
                "has.dot".to_owned(),
                "has space".to_owned(),
                "é".to_owned(),
                "a".repeat(129),
            ] {
                assert_eq!(<$type>::new(invalid), Err(DefinitionError::InvalidRequest));
            }
        }};
    }
    assert_id_contract!(TenantId);
    assert_id_contract!(PrincipalId);
    assert_id_contract!(RequestId);
    assert_id_contract!(CorrelationId);
}

#[test]
fn every_effect_port_receives_the_complete_context_unchanged() {
    let expected = InvocationContextV1::new(
        TenantId::new("tenant-sentinel").expect("tenant"),
        PrincipalId::new("principal-sentinel").expect("principal"),
        RequestId::new("request-sentinel").expect("request"),
        CorrelationId::new("correlation-sentinel").expect("correlation"),
    );
    let mut value = definition(&["allowed"]);
    value.memory = MemoryPolicy {
        enabled: true,
        max_items: 1,
    };
    value.knowledge = KnowledgePolicy {
        enabled: true,
        max_results: 1,
    };
    value.sandbox.allow_execution = true;
    let (provider, _) = RecordingProvider::response(ResponsePlan {
        calls: vec![
            ("allowed".to_owned(), r#"{"input":"x"}"#.to_owned()),
            (
                "factory.memory.recall".to_owned(),
                r#"{"query":"x"}"#.to_owned(),
            ),
            (
                "factory.knowledge.search".to_owned(),
                r#"{"query":"x"}"#.to_owned(),
            ),
            (
                "factory.sandbox.execute".to_owned(),
                r#"{"action":"x","arguments":[]}"#.to_owned(),
            ),
        ],
        ..ResponsePlan::default()
    });
    let counts = Arc::new(EffectCounts::default());
    invoke_contextual(
        expected.clone(),
        value,
        &provider,
        &Controls::new("key"),
        Arc::clone(&counts),
        String::new(),
    )
    .expect("invoke");
    assert_eq!(
        *counts.contexts.lock().expect("contexts"),
        vec![expected; 4]
    );
}

#[test]
fn invocation_context_never_enters_gateway_data_or_scope_digest() {
    let first = InvocationContextV1::new(
        TenantId::new("tenant-sentinel").expect("tenant"),
        PrincipalId::new("principal-sentinel").expect("principal"),
        RequestId::new("request-sentinel").expect("request"),
        CorrelationId::new("correlation-sentinel").expect("correlation"),
    );
    let second = InvocationContextV1::new(
        TenantId::new("other-tenant").expect("tenant"),
        PrincipalId::new("other-principal").expect("principal"),
        RequestId::new("other-request").expect("request"),
        CorrelationId::new("other-correlation").expect("correlation"),
    );
    let (provider, requests) = RecordingProvider::response(ResponsePlan::default());
    let first_result = invoke_contextual(
        first,
        definition(&[]),
        &provider,
        &Controls::new("key-one"),
        Arc::new(EffectCounts::default()),
        String::new(),
    )
    .expect("first");
    let second_result = invoke_contextual(
        second,
        definition(&[]),
        &provider,
        &Controls::new("key-two"),
        Arc::new(EffectCounts::default()),
        String::new(),
    )
    .expect("second");
    assert_eq!(
        first_result.capability_scope_digest,
        second_result.capability_scope_digest
    );
    let snapshot = format!("{:?}", requests.lock().expect("requests"));
    for sentinel in [
        "tenant-sentinel",
        "principal-sentinel",
        "request-sentinel",
        "correlation-sentinel",
        "other-tenant",
        "other-principal",
        "other-request",
        "other-correlation",
    ] {
        assert!(!snapshot.contains(sentinel), "gateway leaked {sentinel}");
    }
}

#[test]
fn in_memory_memory_store_isolates_tenants_without_changing_order_or_query_limits() {
    let mut value = definition(&[]);
    value.memory = MemoryPolicy {
        enabled: true,
        max_items: 2,
    };
    let registry = registry(value);
    let memory = InMemoryMemoryStore::default();
    let tools = FixedToolRegistry::default();
    let knowledge = StaticKnowledgeStore::default();
    let sandbox = DenySandbox;
    let controls = Controls::new("key");
    let tenant_a = InvocationContextV1::new(
        TenantId::new("tenant-a").expect("tenant"),
        PrincipalId::new("principal").expect("principal"),
        RequestId::new("request-a").expect("request"),
        CorrelationId::new("correlation-a").expect("correlation"),
    );
    let tenant_b = InvocationContextV1::new(
        TenantId::new("tenant-b").expect("tenant"),
        PrincipalId::new("principal").expect("principal"),
        RequestId::new("request-b").expect("request"),
        CorrelationId::new("correlation-b").expect("correlation"),
    );
    for (context, stored) in [
        (tenant_a.clone(), "common-a-first"),
        (tenant_b.clone(), "common-b-only"),
        (tenant_a.clone(), "common-a-second"),
    ] {
        let (provider, _) = RecordingProvider::response(ResponsePlan {
            calls: vec![(
                "factory.memory.write".to_owned(),
                format!(r#"{{"value":"{stored}"}}"#),
            )],
            ..ResponsePlan::default()
        });
        let runtime =
            LocalAgentRuntime::new(&registry, &provider, &tools, &memory, &knowledge, &sandbox);
        ready(runtime.invoke(
            context,
            &AgentId::new("agent").expect("id"),
            String::new(),
            controls.control(),
        ))
        .expect("write");
    }
    for (context, expected) in [
        (
            tenant_a,
            vec!["common-a-first".to_owned(), "common-a-second".to_owned()],
        ),
        (tenant_b, vec!["common-b-only".to_owned()]),
    ] {
        let (provider, _) = RecordingProvider::response(ResponsePlan {
            calls: vec![(
                "factory.memory.recall".to_owned(),
                r#"{"query":"common"}"#.to_owned(),
            )],
            ..ResponsePlan::default()
        });
        let runtime =
            LocalAgentRuntime::new(&registry, &provider, &tools, &memory, &knowledge, &sandbox);
        let result = ready(runtime.invoke(
            context,
            &AgentId::new("agent").expect("id"),
            String::new(),
            controls.control(),
        ))
        .expect("recall");
        assert!(matches!(
            &result.events[1],
            InvocationEvent::MemoryRecalled { values } if values == &expected
        ));
    }
}
