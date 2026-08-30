//! Request-relative generation response orchestration.

use crate::{
    FinishReason, GenerateRequest, GenerateResponse, GenerationEvidence, IdempotencyDisposition,
    LlmError, ProviderRequestId, TokenUsage, ToolCall,
    validation::{
        MAX_RESPONSE_TEXT_BYTES, MAX_TOOL_ARGUMENTS_BYTES, MAX_TOOL_CALLS, checked_sum,
        validate_len,
    },
};

#[allow(clippy::too_many_arguments)]
pub(crate) fn assemble_generate_response(
    request: &GenerateRequest,
    text: String,
    tool_calls: Vec<ToolCall>,
    provider_request_id: Option<ProviderRequestId>,
    finish_reason: FinishReason,
    token_usage: Option<TokenUsage>,
    idempotency: IdempotencyDisposition,
) -> Result<GenerateResponse, LlmError> {
    validate_len(text.len(), MAX_RESPONSE_TEXT_BYTES)?;
    validate_len(tool_calls.len(), MAX_TOOL_CALLS)?;
    if tool_calls
        .iter()
        .any(|call| !request.declares_tool(call.name()))
    {
        return Err(LlmError::ProtocolViolation);
    }
    checked_sum(
        tool_calls.iter().map(|call| call.arguments().len()),
        MAX_TOOL_ARGUMENTS_BYTES,
    )?;

    let evidence = GenerationEvidence::new(
        request.provider_id().clone(),
        request.model_id().clone(),
        provider_request_id,
        finish_reason,
        token_usage,
        idempotency,
    );
    Ok(GenerateResponse::from_parts(text, tool_calls, evidence))
}
