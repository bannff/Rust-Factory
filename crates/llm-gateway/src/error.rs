//! Public-safe LLM Gateway error taxonomy.

use std::{error::Error, fmt};

/// Closed error categories exposed by the gateway.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum LlmError {
    InvalidRequest,
    LimitExceeded,
    Unsupported,
    Cancelled,
    DeadlineExceeded,
    Authentication,
    RateLimited,
    Unavailable,
    ProviderRejected,
    ProtocolViolation,
}

impl fmt::Display for LlmError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "invalid request",
            Self::LimitExceeded => "limit exceeded",
            Self::Unsupported => "unsupported operation",
            Self::Cancelled => "operation cancelled",
            Self::DeadlineExceeded => "deadline exceeded",
            Self::Authentication => "authentication failed",
            Self::RateLimited => "rate limited",
            Self::Unavailable => "provider unavailable",
            Self::ProviderRejected => "provider rejected request",
            Self::ProtocolViolation => "provider protocol violation",
        })
    }
}

impl Error for LlmError {}
