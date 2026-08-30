use std::fmt;

/// A closed, data-free failure returned by Knowledge operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum KnowledgeError {
    /// Caller-provided data does not satisfy the contract.
    InvalidRequest,
    /// A fixed resource ceiling or checked arithmetic bound was exceeded.
    LimitExceeded,
    /// The selected index cannot currently serve the operation.
    Unavailable,
    /// An index violated the retrieval protocol.
    ProtocolViolation,
}

impl fmt::Display for KnowledgeError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::InvalidRequest => "invalid_request",
            Self::LimitExceeded => "limit_exceeded",
            Self::Unavailable => "unavailable",
            Self::ProtocolViolation => "protocol_violation",
        })
    }
}

impl std::error::Error for KnowledgeError {}
