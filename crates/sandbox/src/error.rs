use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SandboxError {
    InvalidRequest,
    NotFound,
    Denied,
    LimitExceeded,
    Timeout,
    Unavailable,
    OutcomeUnknown,
    OperationFailed,
}

impl SandboxError {
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidRequest => "invalid_request",
            Self::NotFound => "not_found",
            Self::Denied => "denied",
            Self::LimitExceeded => "limit_exceeded",
            Self::Timeout => "timeout",
            Self::Unavailable => "unavailable",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::OperationFailed => "operation_failed",
        }
    }
}

impl fmt::Display for SandboxError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl std::error::Error for SandboxError {}
