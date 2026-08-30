//! `genai` provider adapter around a composition-injected client.

mod service;

use crate::{
    FinishReason, GenerateRequest, GenerateResponse, IdempotencyDisposition, InvocationControl,
    JsonObject, LlmError, LlmProvider, ProviderFuture, ProviderId, ProviderRequestId, TokenUsage,
    ToolCall, ToolName,
};
use genai::chat::{ChatMessage, ChatOptions, ChatRequest, StopReason, Tool};
use service::{InvocationOutcome, race_invocation};

#[derive(Clone)]
pub struct GenaiProvider {
    client: genai::Client,
    provider_id: ProviderId,
}

impl std::fmt::Debug for GenaiProvider {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("GenaiProvider")
            .field("provider_id", &self.provider_id)
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
#[test]
fn debug_omits_client_configuration() {
    let provider = GenaiProvider::new(
        genai::Client::default(),
        ProviderId::new("safe-provider-sentinel").unwrap(),
    );
    let output = format!("{provider:?}").to_ascii_lowercase();

    assert!(output.contains("safe-provider-sentinel"));
    for label in [
        "client",
        "header",
        "headers",
        "credential",
        "credentials",
        "proxy",
        "endpoint",
        "config",
        "configuration",
        "token",
        "secret",
        "api key",
        "api-key",
        "api_key",
        "apikey",
    ] {
        assert!(!output.contains(label), "debug output contains `{label}`");
    }
}

impl GenaiProvider {
    pub fn new(client: genai::Client, provider_id: ProviderId) -> Self {
        Self {
            client,
            provider_id,
        }
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }
}

impl LlmProvider for GenaiProvider {
    fn generate<'a>(
        &'a self,
        request: &'a GenerateRequest,
        control: InvocationControl<'a>,
    ) -> ProviderFuture<'a> {
        Box::pin(async move {
            control.preflight()?;
            if request.provider_id() != &self.provider_id {
                return Err(LlmError::InvalidRequest);
            }

            let chat_request = to_chat_request(request)?;
            let options = to_chat_options(request);
            let operation =
                self.client
                    .exec_chat(request.model_id().as_str(), chat_request, Some(&options));

            let response = match race_invocation(
                operation,
                control.cancellation.cancelled(),
                control.deadline.elapsed(),
            )
            .await
            {
                InvocationOutcome::Provider(result) => result.map_err(map_error)?,
                InvocationOutcome::Cancelled => return Err(LlmError::Cancelled),
                InvocationOutcome::DeadlineExceeded => return Err(LlmError::DeadlineExceeded),
            };
            normalize_response(request, response)
        })
    }
}

fn to_chat_options(request: &GenerateRequest) -> ChatOptions {
    ChatOptions::default()
        .with_max_tokens(request.limits().max_output_tokens())
        .with_capture_raw_body(false)
}

fn to_chat_request(request: &GenerateRequest) -> Result<ChatRequest, LlmError> {
    let tools = request
        .tools()
        .iter()
        .map(|definition| {
            let schema = serde_json::from_str(definition.input_schema().canonical())
                .map_err(|_| LlmError::InvalidRequest)?;
            Ok(Tool::new(definition.name().as_str())
                .with_description(definition.description())
                .with_schema(schema))
        })
        .collect::<Result<Vec<_>, _>>()?;

    let mut chat_request = ChatRequest::new(vec![ChatMessage::user(request.prompt().input())]);
    chat_request.system = request.prompt().system().map(str::to_owned);
    if !tools.is_empty() {
        chat_request.tools = Some(tools);
    }
    Ok(chat_request)
}

fn normalize_response(
    request: &GenerateRequest,
    response: genai::chat::ChatResponse,
) -> Result<GenerateResponse, LlmError> {
    let text = response.texts().join("");
    let tool_calls = response
        .tool_calls()
        .into_iter()
        .map(|call| {
            let name =
                ToolName::new(call.fn_name.clone()).map_err(|_| LlmError::ProtocolViolation)?;
            let arguments = JsonObject::new(call.fn_arguments.to_string())
                .map_err(|_| LlmError::ProtocolViolation)?;
            ToolCall::new(name, arguments).map_err(|_| LlmError::ProtocolViolation)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let provider_request_id = response
        .response_id
        .map(ProviderRequestId::new)
        .transpose()
        .map_err(|_| LlmError::ProtocolViolation)?;
    let finish_reason = response
        .stop_reason
        .as_ref()
        .map_or(FinishReason::Other, normalize_finish_reason);
    let token_usage = normalize_usage(&response.usage)?;

    GenerateResponse::new(
        request,
        text,
        tool_calls,
        provider_request_id,
        finish_reason,
        token_usage,
        IdempotencyDisposition::Unsupported,
    )
}

fn normalize_finish_reason(reason: &StopReason) -> FinishReason {
    match reason {
        StopReason::Completed(_) | StopReason::StopSequence(_) => FinishReason::Stop,
        StopReason::MaxTokens(_) => FinishReason::Length,
        StopReason::ToolCall(_) => FinishReason::ToolCalls,
        StopReason::ContentFilter(_) => FinishReason::ContentFilter,
        StopReason::Other(_) => FinishReason::Other,
    }
}

fn normalize_usage(usage: &genai::chat::Usage) -> Result<Option<TokenUsage>, LlmError> {
    match (
        usage.prompt_tokens,
        usage.completion_tokens,
        usage.total_tokens,
    ) {
        (None, None, None) => Ok(None),
        (None, None, Some(_)) => Err(LlmError::ProtocolViolation),
        (input, output, total) => {
            // genai normalizes provider-reported zero counters to `None`, so a
            // missing side is zero when the other side proves usage was present.
            let input = u32::try_from(input.unwrap_or_default())
                .map_err(|_| LlmError::ProtocolViolation)?;
            let output = u32::try_from(output.unwrap_or_default())
                .map_err(|_| LlmError::ProtocolViolation)?;
            let total = total
                .map(u32::try_from)
                .transpose()
                .map_err(|_| LlmError::ProtocolViolation)?;
            TokenUsage::new(input, output, total)
                .map(Some)
                .map_err(|error| match error {
                    LlmError::LimitExceeded => LlmError::LimitExceeded,
                    _ => LlmError::ProtocolViolation,
                })
        }
    }
}

fn map_error(error: genai::Error) -> LlmError {
    use genai::Error;

    match error {
        Error::RequiresApiKey { .. } | Error::NoAuthResolver { .. } | Error::NoAuthData { .. } => {
            LlmError::Authentication
        }
        Error::HttpError { status, .. } => map_status(status.as_u16()),
        Error::WebAdapterCall { webc_error, .. } | Error::WebModelCall { webc_error, .. } => {
            map_web_error(&webc_error)
        }
        Error::AdapterNotSupported { .. }
        | Error::MessageRoleNotSupported { .. }
        | Error::MessageContentTypeNotSupported { .. } => LlmError::Unsupported,
        Error::NoChatResponse { .. }
        | Error::InvalidJsonResponseElement { .. }
        | Error::ChatResponseGeneration { .. }
        | Error::ChatResponse { .. }
        | Error::StreamParse { .. }
        | Error::SerdeJson(_)
        | Error::JsonValueExt(_) => LlmError::ProtocolViolation,
        Error::ChatReqHasNoMessages { .. }
        | Error::LastChatMessageIsNotUser { .. }
        | Error::JsonModeWithoutInstruction
        | Error::VerbosityParsing { .. }
        | Error::ReasoningParsingError { .. }
        | Error::ServiceTierParsing { .. }
        | Error::PromptCacheRetentionParsing { .. }
        | Error::AdapterKindMismatch { .. } => LlmError::InvalidRequest,
        Error::WebStream { .. } | Error::Resolver { .. } | Error::ModelMapperFailed { .. } => {
            LlmError::Unavailable
        }
        Error::Internal(_) => LlmError::ProviderRejected,
    }
}

#[allow(clippy::match_wildcard_for_single_variants)]
fn map_web_error(error: &genai::webc::Error) -> LlmError {
    match error {
        genai::webc::Error::ResponseFailedStatus { status, .. } => map_status(status.as_u16()),
        genai::webc::Error::ResponseFailedNotJson { .. }
        | genai::webc::Error::ResponseFailedInvalidJson { .. }
        | genai::webc::Error::JsonValueExt(_) => LlmError::ProtocolViolation,
        _ => LlmError::Unavailable,
    }
}

fn map_status(status: u16) -> LlmError {
    match status {
        401 | 403 => LlmError::Authentication,
        429 => LlmError::RateLimited,
        408 | 500..=599 => LlmError::Unavailable,
        _ => LlmError::ProviderRejected,
    }
}

#[cfg(test)]
mod tests {
    use genai::{
        ModelIden,
        adapter::AdapterKind,
        chat::{ChatResponse, ContentPart, MessageContent, StopReason, Usage},
    };
    use serde_json::json;

    use super::*;
    use crate::{GenerationLimits, ModelId, Prompt, ToolDefinition};

    fn request(tools: Vec<ToolDefinition>) -> GenerateRequest {
        GenerateRequest::new(
            ProviderId::new("provider").unwrap(),
            ModelId::new("gpt-test").unwrap(),
            Prompt::new(Some("system".to_owned()), "input").unwrap(),
            tools,
            GenerationLimits::new(99).unwrap(),
        )
        .unwrap()
    }

    fn response(content: MessageContent, usage: Usage) -> ChatResponse {
        let model = ModelIden::new(AdapterKind::OpenAI, "gpt-test");
        ChatResponse {
            content,
            reasoning_content: Some("must not escape".to_owned()),
            model_iden: model.clone(),
            provider_model_iden: model,
            stop_reason: Some(StopReason::Completed("raw-provider-stop".to_owned())),
            usage,
            captured_raw_body: Some(json!({"secret": "must not escape"})),
            response_id: Some("response-id".to_owned()),
        }
    }

    #[test]
    fn request_options_disable_raw_capture_and_preserve_output_limit() {
        let options = to_chat_options(&request(vec![]));
        assert_eq!(options.max_tokens, Some(99));
        assert_eq!(options.capture_raw_body, Some(false));
        assert!(options.extra_headers.is_none());
        assert!(options.extra_body.is_none());
    }

    #[test]
    fn request_conversion_preserves_prompt_tools_and_schema() {
        let definition = ToolDefinition::new(
            ToolName::new("lookup").unwrap(),
            "description",
            JsonObject::new(r#"{"z":1,"type":"object","a":2}"#).unwrap(),
        )
        .unwrap();
        let converted = to_chat_request(&request(vec![definition])).unwrap();

        assert_eq!(converted.system.as_deref(), Some("system"));
        assert_eq!(converted.messages.len(), 1);
        let tools = converted.tools.as_ref().unwrap();
        assert_eq!(tools.len(), 1);
        assert_eq!(tools[0].name.as_str(), "lookup");
        assert_eq!(tools[0].description.as_deref(), Some("description"));
        assert_eq!(
            tools[0].schema,
            Some(json!({"a": 2, "type": "object", "z": 1}))
        );
    }

    #[test]
    fn response_normalization_projects_only_bounded_normalized_fields() {
        let request = request(vec![]);
        let normalized = normalize_response(
            &request,
            response(
                MessageContent::from_parts(vec![
                    ContentPart::Text("one".to_owned()),
                    ContentPart::Text("two".to_owned()),
                ]),
                Usage {
                    prompt_tokens: Some(2),
                    completion_tokens: Some(3),
                    total_tokens: Some(5),
                    ..Usage::default()
                },
            ),
        )
        .unwrap();

        assert_eq!(normalized.text(), "onetwo");
        assert_eq!(normalized.evidence().provider_id(), request.provider_id());
        assert_eq!(normalized.evidence().model_id(), request.model_id());
        assert_eq!(
            normalized
                .evidence()
                .provider_request_id()
                .unwrap()
                .as_str(),
            "response-id"
        );
        assert_eq!(normalized.evidence().finish_reason(), FinishReason::Stop);
        assert_eq!(
            normalized.evidence().token_usage().unwrap().total_tokens(),
            5
        );
        assert_eq!(
            normalized.evidence().idempotency(),
            IdempotencyDisposition::Unsupported
        );
        assert!(!normalized.text().contains("secret"));
    }

    #[test]
    fn response_normalization_rejects_malformed_tool_outputs() {
        let request = request(vec![]);
        let invalid_name = genai::chat::ToolCall {
            call_id: "id".to_owned(),
            fn_name: "bad name".to_owned(),
            fn_arguments: json!({}),
            thought_signatures: None,
        };
        assert_eq!(
            normalize_response(
                &request,
                response(
                    MessageContent::from_tool_calls(vec![invalid_name]),
                    Usage::default()
                )
            )
            .unwrap_err(),
            LlmError::ProtocolViolation
        );

        let non_object = genai::chat::ToolCall {
            call_id: "id".to_owned(),
            fn_name: "tool".to_owned(),
            fn_arguments: json!([]),
            thought_signatures: None,
        };
        assert_eq!(
            normalize_response(
                &request,
                response(
                    MessageContent::from_tool_calls(vec![non_object]),
                    Usage::default()
                )
            )
            .unwrap_err(),
            LlmError::ProtocolViolation
        );
    }

    #[test]
    fn usage_normalization_handles_absence_zero_missing_sides_and_invalid_totals() {
        assert_eq!(normalize_usage(&Usage::default()).unwrap(), None);
        assert_eq!(
            normalize_usage(&Usage {
                total_tokens: Some(1),
                ..Usage::default()
            })
            .unwrap_err(),
            LlmError::ProtocolViolation
        );
        let output_only = normalize_usage(&Usage {
            completion_tokens: Some(3),
            total_tokens: Some(3),
            ..Usage::default()
        })
        .unwrap()
        .unwrap();
        assert_eq!(output_only.input_tokens(), 0);
        assert_eq!(output_only.output_tokens(), 3);
        assert_eq!(
            normalize_usage(&Usage {
                prompt_tokens: Some(-1),
                completion_tokens: Some(1),
                total_tokens: Some(0),
                ..Usage::default()
            })
            .unwrap_err(),
            LlmError::ProtocolViolation
        );
        assert_eq!(
            normalize_usage(&Usage {
                prompt_tokens: Some(1),
                completion_tokens: Some(1),
                total_tokens: Some(3),
                ..Usage::default()
            })
            .unwrap_err(),
            LlmError::ProtocolViolation
        );
        assert_eq!(
            normalize_usage(&Usage {
                prompt_tokens: Some(1_000_001),
                completion_tokens: Some(0),
                total_tokens: Some(1_000_001),
                ..Usage::default()
            })
            .unwrap_err(),
            LlmError::LimitExceeded
        );
    }

    #[test]
    fn finish_reasons_and_http_statuses_map_to_closed_taxonomies() {
        let reasons = [
            (StopReason::Completed(String::new()), FinishReason::Stop),
            (StopReason::StopSequence(String::new()), FinishReason::Stop),
            (StopReason::MaxTokens(String::new()), FinishReason::Length),
            (StopReason::ToolCall(String::new()), FinishReason::ToolCalls),
            (
                StopReason::ContentFilter(String::new()),
                FinishReason::ContentFilter,
            ),
            (
                StopReason::Other("raw secret".to_owned()),
                FinishReason::Other,
            ),
        ];
        for (reason, expected) in reasons {
            assert_eq!(normalize_finish_reason(&reason), expected);
        }
        for (status, expected) in [
            (401, LlmError::Authentication),
            (403, LlmError::Authentication),
            (429, LlmError::RateLimited),
            (408, LlmError::Unavailable),
            (500, LlmError::Unavailable),
            (599, LlmError::Unavailable),
            (400, LlmError::ProviderRejected),
            (600, LlmError::ProviderRejected),
        ] {
            assert_eq!(map_status(status), expected);
        }
    }

    #[test]
    fn genai_errors_are_safely_normalized_without_raw_details() {
        let model = ModelIden::new(AdapterKind::OpenAI, "gpt-test");
        assert_eq!(
            map_error(genai::Error::RequiresApiKey {
                model_iden: model.clone()
            }),
            LlmError::Authentication
        );
        assert_eq!(
            map_error(genai::Error::AdapterNotSupported {
                adapter_kind: AdapterKind::OpenAI,
                feature: "secret".to_owned()
            }),
            LlmError::Unsupported
        );
        assert_eq!(
            map_error(genai::Error::NoChatResponse { model_iden: model }),
            LlmError::ProtocolViolation
        );
        assert_eq!(
            map_error(genai::Error::Internal("raw secret".to_owned())),
            LlmError::ProviderRejected
        );
    }
}
