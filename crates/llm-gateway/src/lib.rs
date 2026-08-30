#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(
    clippy::missing_errors_doc,
    clippy::must_use_candidate,
    clippy::struct_field_names
)]
//! LLM Gateway capability brick (`family = "llm-gateway"`, `role = "brick"`,
//! `status = "implemented"`).
//!
//! The transport-independent core normalizes bounded non-streaming text
//! generation. Callers own runtimes, deadlines, cancellation wake mechanics,
//! credentials, endpoint policy, concurrency, retries, and process lifecycle.

mod error;
#[cfg(feature = "genai")]
pub mod genai;
mod model;
mod port;
mod service;
#[cfg(feature = "static")]
pub mod r#static;
mod validation;

pub use error::LlmError;
pub use model::{
    FinishReason, GenerateRequest, GenerateResponse, GenerationEvidence, GenerationLimits,
    IdempotencyDisposition, IdempotencyKey, JsonObject, ModelId, Prompt, ProviderId,
    ProviderRequestId, TokenUsage, ToolCall, ToolDefinition, ToolName,
};
pub use port::{
    CancellationFuture, CancellationSignal, DeadlineFactory, DeadlineFuture, DeadlineSignal,
    InvocationControl, LlmProvider, ProviderFuture,
};
pub use validation::{
    MAX_IDENTIFIER_BYTES, MAX_JSON_OBJECT_BYTES, MAX_OUTPUT_TOKENS, MAX_PROMPT_BYTES,
    MAX_PROMPT_TEXT_BYTES, MAX_REPORTED_TOKENS, MAX_RESPONSE_TEXT_BYTES, MAX_TOOL_ARGUMENTS_BYTES,
    MAX_TOOL_CALLS, MAX_TOOL_DESCRIPTION_BYTES, MAX_TOOL_NAME_BYTES, MAX_TOOL_SCHEMAS_BYTES,
    MAX_TOOLS, MAX_TOTAL_TOKENS,
};
