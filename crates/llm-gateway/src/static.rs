//! Deterministic request-independent provider adapter.

use crate::{
    FinishReason, GenerateRequest, GenerateResponse, IdempotencyDisposition, InvocationControl,
    LlmError, LlmProvider, ProviderFuture, ProviderRequestId, TokenUsage, ToolCall,
    validation::{
        MAX_RESPONSE_TEXT_BYTES, MAX_TOOL_ARGUMENTS_BYTES, MAX_TOOL_CALLS, checked_sum,
        validate_len,
    },
};

fn validate_fixture(text: &str, tool_calls: &[ToolCall]) -> Result<(), LlmError> {
    validate_len(text.len(), MAX_RESPONSE_TEXT_BYTES)?;
    validate_len(tool_calls.len(), MAX_TOOL_CALLS)?;
    checked_sum(
        tool_calls.iter().map(|call| call.arguments().len()),
        MAX_TOOL_ARGUMENTS_BYTES,
    )?;
    Ok(())
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StaticFixture {
    text: String,
    tool_calls: Vec<ToolCall>,
    provider_request_id: Option<ProviderRequestId>,
    finish_reason: FinishReason,
    token_usage: Option<TokenUsage>,
    idempotency: IdempotencyDisposition,
}

impl StaticFixture {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        text: impl Into<String>,
        tool_calls: Vec<ToolCall>,
        provider_request_id: Option<ProviderRequestId>,
        finish_reason: FinishReason,
        token_usage: Option<TokenUsage>,
        idempotency: IdempotencyDisposition,
    ) -> Result<Self, LlmError> {
        let text = text.into();
        validate_fixture(&text, &tool_calls)?;
        Ok(Self {
            text,
            tool_calls,
            provider_request_id,
            finish_reason,
            token_usage,
            idempotency,
        })
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn tool_calls(&self) -> &[ToolCall] {
        &self.tool_calls
    }

    pub fn provider_request_id(&self) -> Option<&ProviderRequestId> {
        self.provider_request_id.as_ref()
    }

    pub fn finish_reason(&self) -> FinishReason {
        self.finish_reason
    }

    pub fn token_usage(&self) -> Option<TokenUsage> {
        self.token_usage
    }

    pub fn idempotency(&self) -> IdempotencyDisposition {
        self.idempotency
    }
}

#[derive(Clone, Debug)]
pub struct StaticProvider {
    result: Result<StaticFixture, LlmError>,
}

impl StaticProvider {
    pub fn new(result: Result<StaticFixture, LlmError>) -> Self {
        Self { result }
    }

    pub fn success(fixture: StaticFixture) -> Self {
        Self::new(Ok(fixture))
    }

    pub fn error(error: LlmError) -> Self {
        Self::new(Err(error))
    }
}

impl LlmProvider for StaticProvider {
    fn generate<'a>(
        &'a self,
        request: &'a GenerateRequest,
        control: InvocationControl<'a>,
    ) -> ProviderFuture<'a> {
        Box::pin(async move {
            control.preflight()?;
            let fixture = self.result.as_ref().map_err(|error| *error)?;
            GenerateResponse::new(
                request,
                fixture.text.clone(),
                fixture.tool_calls.clone(),
                fixture.provider_request_id.clone(),
                fixture.finish_reason,
                fixture.token_usage,
                fixture.idempotency,
            )
        })
    }
}
