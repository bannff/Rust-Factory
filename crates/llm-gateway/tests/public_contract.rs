use std::{
    error::Error,
    future::{Future, pending, ready},
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
    task::{Context, Poll},
    time::Instant,
};

use llm_gateway::{
    CancellationFuture, CancellationSignal, DeadlineFactory, DeadlineFuture, DeadlineSignal,
    FinishReason, GenerateRequest, GenerateResponse, GenerationLimits, IdempotencyDisposition,
    IdempotencyKey, InvocationControl, JsonObject, LlmError, LlmProvider, MAX_IDENTIFIER_BYTES,
    MAX_JSON_OBJECT_BYTES, MAX_OUTPUT_TOKENS, MAX_PROMPT_TEXT_BYTES, MAX_REPORTED_TOKENS,
    MAX_RESPONSE_TEXT_BYTES, MAX_TOOL_CALLS, MAX_TOOL_DESCRIPTION_BYTES, MAX_TOOL_NAME_BYTES,
    MAX_TOOLS, ModelId, Prompt, ProviderFuture, ProviderId, ProviderRequestId, TokenUsage,
    ToolCall, ToolDefinition, ToolName,
};

fn poll_once<F: Future + ?Sized>(future: Pin<&mut F>) -> Poll<F::Output> {
    future.poll(&mut Context::from_waker(std::task::Waker::noop()))
}

#[cfg(feature = "static")]
fn run_ready<F: Future>(future: F) -> F::Output {
    let mut future = std::pin::pin!(future);
    match poll_once(future.as_mut()) {
        Poll::Ready(output) => output,
        Poll::Pending => panic!("test future unexpectedly pending"),
    }
}

#[derive(Default)]
struct Signal {
    set: AtomicBool,
    instant: Option<Instant>,
}

impl Signal {
    fn set(value: bool) -> Self {
        Self {
            set: AtomicBool::new(value),
            instant: None,
        }
    }
}

impl CancellationSignal for Signal {
    fn is_cancelled(&self) -> bool {
        self.set.load(Ordering::SeqCst)
    }

    fn cancelled(&self) -> CancellationFuture<'_> {
        if self.is_cancelled() {
            Box::pin(ready(()))
        } else {
            Box::pin(pending())
        }
    }
}

impl DeadlineSignal for Signal {
    fn instant(&self) -> Instant {
        self.instant.unwrap_or_else(Instant::now)
    }

    fn is_elapsed(&self) -> bool {
        self.set.load(Ordering::SeqCst)
    }

    fn elapsed(&self) -> DeadlineFuture<'_> {
        if self.is_elapsed() {
            Box::pin(ready(()))
        } else {
            Box::pin(pending())
        }
    }
}

struct Factory;
impl DeadlineFactory for Factory {
    fn create(&self, instant: Instant) -> Box<dyn DeadlineSignal> {
        Box::new(Signal {
            set: AtomicBool::new(false),
            instant: Some(instant),
        })
    }
}

fn control<'a>(
    key: &'a IdempotencyKey,
    cancel: &'a Signal,
    deadline: &'a Signal,
) -> InvocationControl<'a> {
    InvocationControl {
        idempotency_key: key,
        cancellation: cancel,
        deadline,
    }
}

fn object_schema() -> JsonObject {
    JsonObject::new(r#"{"type":"object"}"#).unwrap()
}

fn tool(name: &str, schema: JsonObject) -> ToolDefinition {
    ToolDefinition::new(ToolName::new(name).unwrap(), "description", schema).unwrap()
}

fn request_with_tools(tools: Vec<ToolDefinition>) -> GenerateRequest {
    GenerateRequest::new(
        ProviderId::new("provider").unwrap(),
        ModelId::new("model").unwrap(),
        Prompt::new(None, "input").unwrap(),
        tools,
        GenerationLimits::new(128).unwrap(),
    )
    .unwrap()
}

fn padded_object(target: usize, schema: bool) -> JsonObject {
    let template = if schema {
        r#"{"pad":"","type":"object"}"#
    } else {
        r#"{"pad":""}"#
    };
    let overhead = template.len();
    assert!(target >= overhead);
    let input = if schema {
        format!(
            r#"{{"pad":"{}","type":"object"}}"#,
            "x".repeat(target - overhead)
        )
    } else {
        format!(r#"{{"pad":"{}"}}"#, "x".repeat(target - overhead))
    };
    let object = JsonObject::new(input).unwrap();
    assert_eq!(object.len(), target);
    object
}

#[test]
fn identifiers_enforce_nonempty_and_utf8_byte_ceiling() {
    assert_eq!(ProviderId::new("").unwrap_err(), LlmError::InvalidRequest);
    assert!(ModelId::new("é".repeat(MAX_IDENTIFIER_BYTES / 2)).is_ok());
    assert_eq!(
        ModelId::new(format!("{}a", "é".repeat(MAX_IDENTIFIER_BYTES / 2))).unwrap_err(),
        LlmError::LimitExceeded
    );
    assert!(IdempotencyKey::new("i".repeat(MAX_IDENTIFIER_BYTES)).is_ok());
    assert_eq!(
        ProviderRequestId::new("i".repeat(MAX_IDENTIFIER_BYTES + 1)).unwrap_err(),
        LlmError::LimitExceeded
    );
}

#[test]
fn tool_name_grammar_and_utf8_limits_are_closed() {
    for valid in ["a", "Z9", "a_b.c:d-e"] {
        assert_eq!(ToolName::new(valid).unwrap().as_str(), valid);
    }
    for invalid in ["", "_a", "-a", "a/b", "a b", "é", "a\n"] {
        assert_eq!(
            ToolName::new(invalid).unwrap_err(),
            LlmError::InvalidRequest,
            "{invalid:?}"
        );
    }
    assert!(ToolName::new("a".repeat(MAX_TOOL_NAME_BYTES)).is_ok());
    assert_eq!(
        ToolName::new("a".repeat(MAX_TOOL_NAME_BYTES + 1)).unwrap_err(),
        LlmError::LimitExceeded
    );
}

#[test]
fn prompt_accepts_empty_input_and_enforces_each_utf8_byte_limit() {
    assert_eq!(Prompt::new(None, "").unwrap().input(), "");
    assert!(
        Prompt::new(
            Some("é".repeat(MAX_PROMPT_TEXT_BYTES / 2)),
            "x".repeat(MAX_PROMPT_TEXT_BYTES)
        )
        .is_ok()
    );
    assert_eq!(
        Prompt::new(None, format!("{}a", "é".repeat(MAX_PROMPT_TEXT_BYTES / 2))).unwrap_err(),
        LlmError::LimitExceeded
    );
    assert_eq!(
        Prompt::new(Some("x".repeat(MAX_PROMPT_TEXT_BYTES + 1)), "").unwrap_err(),
        LlmError::LimitExceeded
    );
}

#[test]
fn json_rejects_duplicate_nested_trailing_malformed_and_non_object_values() {
    for invalid in [
        r#"{"a":{"x":1,"x":2}}"#,
        r#"{"a":1} trailing"#,
        r#"{"a":}"#,
        "[]",
        "null",
        "\"text\"",
        "1",
    ] {
        assert_eq!(
            JsonObject::new(invalid).unwrap_err(),
            LlmError::InvalidRequest,
            "{invalid}"
        );
    }
}

#[test]
fn json_canonicalizes_recursively_and_enforces_canonical_byte_limit() {
    let object = JsonObject::new(r#" { "z":{"b":2,"a":1}, "a":[{"d":4,"c":3}] } "#).unwrap();
    assert_eq!(
        object.canonical(),
        r#"{"a":[{"c":3,"d":4}],"z":{"a":1,"b":2}}"#
    );
    assert_eq!(
        padded_object(MAX_JSON_OBJECT_BYTES, false).len(),
        MAX_JSON_OBJECT_BYTES
    );
    let overhead = r#"{"pad":""}"#.len();
    let oversized = format!(
        r#"{{"pad":"{}"}}"#,
        "x".repeat(MAX_JSON_OBJECT_BYTES + 1 - overhead)
    );
    assert_eq!(
        JsonObject::new(oversized).unwrap_err(),
        LlmError::LimitExceeded
    );
}

#[test]
fn schema_requires_exact_top_level_string_type_object() {
    assert!(ToolDefinition::new(ToolName::new("t").unwrap(), "", object_schema()).is_ok());
    for schema in [
        r"{}",
        r#"{"type":"array"}"#,
        r#"{"type":null}"#,
        r#"{"type":["object"]}"#,
        r#"{"type":{"const":"object"}}"#,
        r#"{"properties":{"type":{"const":"object"}}}"#,
    ] {
        assert_eq!(
            ToolDefinition::new(
                ToolName::new("t").unwrap(),
                "",
                JsonObject::new(schema).unwrap()
            )
            .unwrap_err(),
            LlmError::Unsupported,
            "{schema}"
        );
    }
}

#[test]
fn tool_description_and_request_count_and_schema_aggregate_limits_are_exact() {
    assert!(
        ToolDefinition::new(
            ToolName::new("t").unwrap(),
            "é".repeat(MAX_TOOL_DESCRIPTION_BYTES / 2),
            object_schema()
        )
        .is_ok()
    );
    assert_eq!(
        ToolDefinition::new(
            ToolName::new("t").unwrap(),
            format!("{}a", "é".repeat(MAX_TOOL_DESCRIPTION_BYTES / 2)),
            object_schema()
        )
        .unwrap_err(),
        LlmError::LimitExceeded
    );

    let exact_count = (0..MAX_TOOLS)
        .map(|i| tool(&format!("t{i}"), object_schema()))
        .collect();
    assert!(
        GenerateRequest::new(
            ProviderId::new("p").unwrap(),
            ModelId::new("m").unwrap(),
            Prompt::new(None, "").unwrap(),
            exact_count,
            GenerationLimits::new(1).unwrap()
        )
        .is_ok()
    );
    let over_count = (0..=MAX_TOOLS)
        .map(|i| tool(&format!("t{i}"), object_schema()))
        .collect();
    assert_eq!(
        GenerateRequest::new(
            ProviderId::new("p").unwrap(),
            ModelId::new("m").unwrap(),
            Prompt::new(None, "").unwrap(),
            over_count,
            GenerationLimits::new(1).unwrap()
        )
        .unwrap_err(),
        LlmError::LimitExceeded
    );

    let exact_aggregate = (0..4)
        .map(|i| tool(&format!("s{i}"), padded_object(MAX_JSON_OBJECT_BYTES, true)))
        .collect();
    assert_eq!(request_with_tools(exact_aggregate).tools().len(), 4);
    let over_aggregate = (0..4)
        .map(|i| tool(&format!("s{i}"), padded_object(MAX_JSON_OBJECT_BYTES, true)))
        .chain([tool("extra", object_schema())])
        .collect();
    assert_eq!(
        GenerateRequest::new(
            ProviderId::new("p").unwrap(),
            ModelId::new("m").unwrap(),
            Prompt::new(None, "").unwrap(),
            over_aggregate,
            GenerationLimits::new(1).unwrap()
        )
        .unwrap_err(),
        LlmError::LimitExceeded
    );
}

#[test]
fn duplicate_tools_are_rejected() {
    assert_eq!(
        GenerateRequest::new(
            ProviderId::new("p").unwrap(),
            ModelId::new("m").unwrap(),
            Prompt::new(None, "").unwrap(),
            vec![tool("same", object_schema()), tool("same", object_schema())],
            GenerationLimits::new(1).unwrap(),
        )
        .unwrap_err(),
        LlmError::InvalidRequest
    );
}

#[test]
fn generation_and_token_limits_are_exact_and_totals_are_consistent() {
    assert_eq!(
        GenerationLimits::new(0).unwrap_err(),
        LlmError::LimitExceeded
    );
    assert!(GenerationLimits::new(MAX_OUTPUT_TOKENS).is_ok());
    assert_eq!(
        GenerationLimits::new(MAX_OUTPUT_TOKENS + 1).unwrap_err(),
        LlmError::LimitExceeded
    );

    let usage = TokenUsage::new(MAX_REPORTED_TOKENS, MAX_REPORTED_TOKENS, Some(2_000_000)).unwrap();
    assert_eq!(usage.total_tokens(), 2_000_000);
    assert_eq!(
        TokenUsage::new(MAX_REPORTED_TOKENS + 1, 0, None).unwrap_err(),
        LlmError::LimitExceeded
    );
    assert_eq!(
        TokenUsage::new(1, 2, Some(4)).unwrap_err(),
        LlmError::ProtocolViolation
    );
    assert_eq!(TokenUsage::new(0, 0, Some(0)).unwrap().total_tokens(), 0);
}

#[test]
fn response_enforces_text_and_call_count_limits() {
    let request = request_with_tools(vec![tool("declared", object_schema())]);
    assert!(
        GenerateResponse::new(
            &request,
            "é".repeat(MAX_RESPONSE_TEXT_BYTES / 2),
            vec![],
            None,
            FinishReason::Stop,
            None,
            IdempotencyDisposition::Unsupported
        )
        .is_ok()
    );
    assert_eq!(
        GenerateResponse::new(
            &request,
            format!("{}a", "é".repeat(MAX_RESPONSE_TEXT_BYTES / 2)),
            vec![],
            None,
            FinishReason::Stop,
            None,
            IdempotencyDisposition::Unsupported
        )
        .unwrap_err(),
        LlmError::LimitExceeded
    );

    let call = || {
        ToolCall::new(
            ToolName::new("declared").unwrap(),
            JsonObject::new("{}").unwrap(),
        )
        .unwrap()
    };
    assert!(
        GenerateResponse::new(
            &request,
            "",
            (0..MAX_TOOL_CALLS).map(|_| call()).collect(),
            None,
            FinishReason::ToolCalls,
            None,
            IdempotencyDisposition::Unsupported
        )
        .is_ok()
    );
    assert_eq!(
        GenerateResponse::new(
            &request,
            "",
            (0..=MAX_TOOL_CALLS).map(|_| call()).collect(),
            None,
            FinishReason::ToolCalls,
            None,
            IdempotencyDisposition::Unsupported
        )
        .unwrap_err(),
        LlmError::LimitExceeded
    );
}

#[test]
fn response_rejects_undeclared_calls() {
    let request = request_with_tools(vec![tool("declared", object_schema())]);
    let undeclared = ToolCall::new(
        ToolName::new("other").unwrap(),
        JsonObject::new("{}").unwrap(),
    )
    .unwrap();
    assert_eq!(
        GenerateResponse::new(
            &request,
            "",
            vec![undeclared],
            None,
            FinishReason::ToolCalls,
            None,
            IdempotencyDisposition::Unsupported
        )
        .unwrap_err(),
        LlmError::ProtocolViolation
    );
}

#[test]
fn response_enforces_tool_argument_aggregate_limit() {
    let request = request_with_tools(vec![tool("declared", object_schema())]);
    let call = || {
        ToolCall::new(
            ToolName::new("declared").unwrap(),
            JsonObject::new("{}").unwrap(),
        )
        .unwrap()
    };
    let exact_args = (0..4)
        .map(|_| {
            ToolCall::new(
                ToolName::new("declared").unwrap(),
                padded_object(MAX_JSON_OBJECT_BYTES, false),
            )
            .unwrap()
        })
        .collect();
    assert!(
        GenerateResponse::new(
            &request,
            "",
            exact_args,
            None,
            FinishReason::ToolCalls,
            None,
            IdempotencyDisposition::Unsupported
        )
        .is_ok()
    );
    let over_args = (0..4)
        .map(|_| {
            ToolCall::new(
                ToolName::new("declared").unwrap(),
                padded_object(MAX_JSON_OBJECT_BYTES, false),
            )
            .unwrap()
        })
        .chain([call()])
        .collect();
    assert_eq!(
        GenerateResponse::new(
            &request,
            "",
            over_args,
            None,
            FinishReason::ToolCalls,
            None,
            IdempotencyDisposition::Unsupported
        )
        .unwrap_err(),
        LlmError::LimitExceeded
    );
}

#[test]
fn response_evidence_identity_is_derived_from_request() {
    let request = request_with_tools(vec![]);
    let response = GenerateResponse::new(
        &request,
        "ok",
        vec![],
        Some(ProviderRequestId::new("request-id").unwrap()),
        FinishReason::Other,
        Some(TokenUsage::new(2, 3, Some(5)).unwrap()),
        IdempotencyDisposition::Unsupported,
    )
    .unwrap();
    assert_eq!(response.evidence().provider_id(), request.provider_id());
    assert_eq!(response.evidence().model_id(), request.model_id());
    assert_eq!(
        response.evidence().provider_request_id().unwrap().as_str(),
        "request-id"
    );
    assert_eq!(response.evidence().finish_reason(), FinishReason::Other);
    assert_eq!(response.evidence().token_usage().unwrap().total_tokens(), 5);
    assert_eq!(
        response.evidence().idempotency(),
        IdempotencyDisposition::Unsupported
    );
}

#[test]
fn errors_are_closed_safe_messages_without_sources() {
    let cases = [
        (LlmError::InvalidRequest, "invalid request"),
        (LlmError::LimitExceeded, "limit exceeded"),
        (LlmError::Unsupported, "unsupported operation"),
        (LlmError::Cancelled, "operation cancelled"),
        (LlmError::DeadlineExceeded, "deadline exceeded"),
        (LlmError::Authentication, "authentication failed"),
        (LlmError::RateLimited, "rate limited"),
        (LlmError::Unavailable, "provider unavailable"),
        (LlmError::ProviderRejected, "provider rejected request"),
        (LlmError::ProtocolViolation, "provider protocol violation"),
    ];
    for (error, display) in cases {
        assert_eq!(error.to_string(), display);
        assert!(error.source().is_none());
        assert!(!error.to_string().contains("secret"));
    }
}

#[test]
fn preflight_prioritizes_cancellation_over_deadline() {
    let key = IdempotencyKey::new("key").unwrap();
    let both = Signal::set(true);
    assert_eq!(
        control(&key, &both, &both).preflight().unwrap_err(),
        LlmError::Cancelled
    );
    let clear = Signal::set(false);
    assert_eq!(
        control(&key, &clear, &both).preflight().unwrap_err(),
        LlmError::DeadlineExceeded
    );
    assert!(control(&key, &clear, &clear).preflight().is_ok());
}

struct PendingProvider {
    drops: Arc<AtomicUsize>,
}
struct DropFuture(Arc<AtomicUsize>);
impl Future for DropFuture {
    type Output = Result<GenerateResponse, LlmError>;
    fn poll(self: Pin<&mut Self>, _context: &mut Context<'_>) -> Poll<Self::Output> {
        Poll::Pending
    }
}
impl Drop for DropFuture {
    fn drop(&mut self) {
        self.0.fetch_add(1, Ordering::SeqCst);
    }
}
impl LlmProvider for PendingProvider {
    fn generate<'a>(
        &'a self,
        _request: &'a GenerateRequest,
        _control: InvocationControl<'a>,
    ) -> ProviderFuture<'a> {
        Box::pin(DropFuture(Arc::clone(&self.drops)))
    }
}

#[test]
fn traits_are_object_safe_and_pending_provider_future_is_cancelled_by_drop() {
    let factory: &dyn DeadlineFactory = &Factory;
    let deadline = factory.create(Instant::now());
    let deadline_object: &dyn DeadlineSignal = deadline.as_ref();
    assert_eq!(deadline_object.instant(), deadline.instant());

    let cancellation = Signal::set(false);
    let cancellation_object: &dyn CancellationSignal = &cancellation;
    assert!(!cancellation_object.is_cancelled());

    let drops = Arc::new(AtomicUsize::new(0));
    let provider = PendingProvider {
        drops: Arc::clone(&drops),
    };
    let provider_object: &dyn LlmProvider = &provider;
    let request = request_with_tools(vec![]);
    let key = IdempotencyKey::new("key").unwrap();
    let mut future = provider_object.generate(
        &request,
        InvocationControl {
            idempotency_key: &key,
            cancellation: cancellation_object,
            deadline: deadline_object,
        },
    );
    assert!(poll_once(future.as_mut()).is_pending());
    drop(future);
    assert_eq!(drops.load(Ordering::SeqCst), 1);
}

#[cfg(feature = "static")]
#[test]
fn static_provider_is_request_relative_and_preflight_precedes_configured_error() {
    use llm_gateway::r#static::{StaticFixture, StaticProvider};

    let fixture = StaticFixture::new(
        "ok",
        vec![
            ToolCall::new(
                ToolName::new("allowed").unwrap(),
                JsonObject::new("{}").unwrap(),
            )
            .unwrap(),
        ],
        Some(ProviderRequestId::new("provider-request").unwrap()),
        FinishReason::ToolCalls,
        None,
        IdempotencyDisposition::Unsupported,
    )
    .unwrap();
    let provider = StaticProvider::success(fixture);
    let request = GenerateRequest::new(
        ProviderId::new("actual-provider").unwrap(),
        ModelId::new("actual-model").unwrap(),
        Prompt::new(None, "").unwrap(),
        vec![tool("allowed", object_schema())],
        GenerationLimits::new(1).unwrap(),
    )
    .unwrap();
    let key = IdempotencyKey::new("key").unwrap();
    let clear = Signal::set(false);
    let response = run_ready(provider.generate(&request, control(&key, &clear, &clear))).unwrap();
    assert_eq!(
        response.evidence().provider_id().as_str(),
        "actual-provider"
    );
    assert_eq!(response.evidence().model_id().as_str(), "actual-model");

    let other_request = request_with_tools(vec![]);
    assert_eq!(
        run_ready(provider.generate(&other_request, control(&key, &clear, &clear))).unwrap_err(),
        LlmError::ProtocolViolation
    );

    let configured_error = StaticProvider::error(LlmError::Unavailable);
    let cancelled = Signal::set(true);
    assert_eq!(
        run_ready(configured_error.generate(&request, control(&key, &cancelled, &clear)))
            .unwrap_err(),
        LlmError::Cancelled
    );
    assert_eq!(
        run_ready(configured_error.generate(&request, control(&key, &clear, &clear))).unwrap_err(),
        LlmError::Unavailable
    );
}

#[cfg(feature = "static")]
#[test]
fn static_fixture_enforces_request_independent_output_bounds() {
    use llm_gateway::r#static::StaticFixture;

    assert!(
        StaticFixture::new(
            "x".repeat(MAX_RESPONSE_TEXT_BYTES),
            vec![],
            None,
            FinishReason::Stop,
            None,
            IdempotencyDisposition::Unsupported
        )
        .is_ok()
    );
    assert_eq!(
        StaticFixture::new(
            "x".repeat(MAX_RESPONSE_TEXT_BYTES + 1),
            vec![],
            None,
            FinishReason::Stop,
            None,
            IdempotencyDisposition::Unsupported
        )
        .unwrap_err(),
        LlmError::LimitExceeded
    );
    let calls = (0..=MAX_TOOL_CALLS)
        .map(|_| {
            ToolCall::new(ToolName::new("t").unwrap(), JsonObject::new("{}").unwrap()).unwrap()
        })
        .collect();
    assert_eq!(
        StaticFixture::new(
            "",
            calls,
            None,
            FinishReason::ToolCalls,
            None,
            IdempotencyDisposition::Unsupported
        )
        .unwrap_err(),
        LlmError::LimitExceeded
    );
}
