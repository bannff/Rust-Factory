//! Validated provider-neutral generation models.

use std::collections::HashSet;

use crate::{
    LlmError,
    service::assemble_generate_response,
    validation::{
        MAX_JSON_OBJECT_BYTES, MAX_OUTPUT_TOKENS, MAX_PROMPT_BYTES, MAX_PROMPT_TEXT_BYTES,
        MAX_REPORTED_TOKENS, MAX_TOOL_DESCRIPTION_BYTES, MAX_TOOL_SCHEMAS_BYTES, MAX_TOOLS,
        MAX_TOTAL_TOKENS, canonical_object, checked_sum, validate_identifier, validate_len,
        validate_object_schema, validate_tool_name,
    },
};

macro_rules! identifier {
    ($name:ident) => {
        #[derive(Clone, Debug, Eq, Hash, PartialEq)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, LlmError> {
                let value = value.into();
                validate_identifier(&value)?;
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl AsRef<str> for $name {
            fn as_ref(&self) -> &str {
                self.as_str()
            }
        }
    };
}

identifier!(ModelId);
identifier!(ProviderId);
identifier!(IdempotencyKey);
identifier!(ProviderRequestId);

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ToolName(String);

impl ToolName {
    pub fn new(value: impl Into<String>) -> Result<Self, LlmError> {
        let value = value.into();
        validate_tool_name(&value)?;
        Ok(Self(value))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ToolName {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Prompt {
    system: Option<String>,
    input: String,
}

impl Prompt {
    pub fn new(system: Option<String>, input: impl Into<String>) -> Result<Self, LlmError> {
        let input = input.into();
        validate_len(input.len(), MAX_PROMPT_TEXT_BYTES)?;
        if let Some(system) = &system {
            validate_len(system.len(), MAX_PROMPT_TEXT_BYTES)?;
        }
        checked_sum(
            [system.as_ref().map_or(0, String::len), input.len()],
            MAX_PROMPT_BYTES,
        )?;
        Ok(Self { system, input })
    }

    pub fn system(&self) -> Option<&str> {
        self.system.as_deref()
    }

    pub fn input(&self) -> &str {
        &self.input
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct JsonObject {
    canonical: String,
}

impl JsonObject {
    pub fn new(input: impl AsRef<str>) -> Result<Self, LlmError> {
        Ok(Self {
            canonical: canonical_object(input.as_ref())?,
        })
    }

    pub fn canonical(&self) -> &str {
        &self.canonical
    }

    pub fn len(&self) -> usize {
        self.canonical.len()
    }

    pub fn is_empty(&self) -> bool {
        false
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolDefinition {
    name: ToolName,
    description: String,
    input_schema: JsonObject,
}

impl ToolDefinition {
    pub fn new(
        name: ToolName,
        description: impl Into<String>,
        input_schema: JsonObject,
    ) -> Result<Self, LlmError> {
        let description = description.into();
        validate_len(description.len(), MAX_TOOL_DESCRIPTION_BYTES)?;
        validate_object_schema(input_schema.canonical())?;
        Ok(Self {
            name,
            description,
            input_schema,
        })
    }

    pub fn name(&self) -> &ToolName {
        &self.name
    }

    pub fn description(&self) -> &str {
        &self.description
    }

    pub fn input_schema(&self) -> &JsonObject {
        &self.input_schema
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct GenerationLimits {
    max_output_tokens: u32,
}

impl GenerationLimits {
    pub fn new(max_output_tokens: u32) -> Result<Self, LlmError> {
        if !(1..=MAX_OUTPUT_TOKENS).contains(&max_output_tokens) {
            return Err(LlmError::LimitExceeded);
        }
        Ok(Self { max_output_tokens })
    }

    pub fn max_output_tokens(self) -> u32 {
        self.max_output_tokens
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerateRequest {
    provider_id: ProviderId,
    model_id: ModelId,
    prompt: Prompt,
    tools: Vec<ToolDefinition>,
    limits: GenerationLimits,
}

impl GenerateRequest {
    pub fn new(
        provider_id: ProviderId,
        model_id: ModelId,
        prompt: Prompt,
        tools: Vec<ToolDefinition>,
        limits: GenerationLimits,
    ) -> Result<Self, LlmError> {
        validate_len(tools.len(), MAX_TOOLS)?;
        let mut names = HashSet::with_capacity(tools.len());
        if tools.iter().any(|tool| !names.insert(tool.name().as_str())) {
            return Err(LlmError::InvalidRequest);
        }
        checked_sum(
            tools.iter().map(|tool| tool.input_schema().len()),
            MAX_TOOL_SCHEMAS_BYTES,
        )?;
        Ok(Self {
            provider_id,
            model_id,
            prompt,
            tools,
            limits,
        })
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    pub fn model_id(&self) -> &ModelId {
        &self.model_id
    }

    pub fn prompt(&self) -> &Prompt {
        &self.prompt
    }

    pub fn tools(&self) -> &[ToolDefinition] {
        &self.tools
    }

    pub fn limits(&self) -> GenerationLimits {
        self.limits
    }

    pub(crate) fn declares_tool(&self, name: &ToolName) -> bool {
        self.tools.iter().any(|tool| tool.name() == name)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ToolCall {
    name: ToolName,
    arguments: JsonObject,
}

impl ToolCall {
    pub fn new(name: ToolName, arguments: JsonObject) -> Result<Self, LlmError> {
        validate_len(arguments.len(), MAX_JSON_OBJECT_BYTES)?;
        Ok(Self { name, arguments })
    }

    pub fn name(&self) -> &ToolName {
        &self.name
    }

    pub fn arguments(&self) -> &JsonObject {
        &self.arguments
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct TokenUsage {
    input_tokens: u32,
    output_tokens: u32,
    total_tokens: u32,
}

impl TokenUsage {
    pub fn new(
        input_tokens: u32,
        output_tokens: u32,
        reported_total: Option<u32>,
    ) -> Result<Self, LlmError> {
        if input_tokens > MAX_REPORTED_TOKENS || output_tokens > MAX_REPORTED_TOKENS {
            return Err(LlmError::LimitExceeded);
        }
        let total_tokens = input_tokens
            .checked_add(output_tokens)
            .ok_or(LlmError::LimitExceeded)?;
        if total_tokens > MAX_TOTAL_TOKENS {
            return Err(LlmError::LimitExceeded);
        }
        if reported_total.is_some_and(|reported| reported != total_tokens) {
            return Err(LlmError::ProtocolViolation);
        }
        Ok(Self {
            input_tokens,
            output_tokens,
            total_tokens,
        })
    }

    pub fn input_tokens(self) -> u32 {
        self.input_tokens
    }

    pub fn output_tokens(self) -> u32 {
        self.output_tokens
    }

    pub fn total_tokens(self) -> u32 {
        self.total_tokens
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Other,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IdempotencyDisposition {
    Unsupported,
    Accepted,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerationEvidence {
    provider_id: ProviderId,
    model_id: ModelId,
    provider_request_id: Option<ProviderRequestId>,
    finish_reason: FinishReason,
    token_usage: Option<TokenUsage>,
    idempotency: IdempotencyDisposition,
}

impl GenerationEvidence {
    pub(crate) fn new(
        provider_id: ProviderId,
        model_id: ModelId,
        provider_request_id: Option<ProviderRequestId>,
        finish_reason: FinishReason,
        token_usage: Option<TokenUsage>,
        idempotency: IdempotencyDisposition,
    ) -> Self {
        Self {
            provider_id,
            model_id,
            provider_request_id,
            finish_reason,
            token_usage,
            idempotency,
        }
    }

    pub fn provider_id(&self) -> &ProviderId {
        &self.provider_id
    }

    pub fn model_id(&self) -> &ModelId {
        &self.model_id
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct GenerateResponse {
    text: String,
    tool_calls: Vec<ToolCall>,
    evidence: GenerationEvidence,
}

impl GenerateResponse {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        request: &GenerateRequest,
        text: impl Into<String>,
        tool_calls: Vec<ToolCall>,
        provider_request_id: Option<ProviderRequestId>,
        finish_reason: FinishReason,
        token_usage: Option<TokenUsage>,
        idempotency: IdempotencyDisposition,
    ) -> Result<Self, LlmError> {
        assemble_generate_response(
            request,
            text.into(),
            tool_calls,
            provider_request_id,
            finish_reason,
            token_usage,
            idempotency,
        )
    }

    pub(crate) fn from_parts(
        text: String,
        tool_calls: Vec<ToolCall>,
        evidence: GenerationEvidence,
    ) -> Self {
        Self {
            text,
            tool_calls,
            evidence,
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }

    pub fn tool_calls(&self) -> &[ToolCall] {
        &self.tool_calls
    }

    pub fn evidence(&self) -> &GenerationEvidence {
        &self.evidence
    }
}
