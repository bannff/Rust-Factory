use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicErrorCode {
    InvalidRequest,
    InvalidDefinition,
    NotFound,
    Conflict,
    LimitExceeded,
    OperationFailed,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum EvaluationError {
    InvalidRequest,
    InvalidDefinition,
    NotFound,
    Conflict,
    LimitExceeded,
    MalformedEvidence,
    AdapterFailure,
}
impl EvaluationError {
    #[must_use]
    pub const fn public_code(&self) -> PublicErrorCode {
        match self {
            Self::InvalidRequest => PublicErrorCode::InvalidRequest,
            Self::InvalidDefinition => PublicErrorCode::InvalidDefinition,
            Self::NotFound => PublicErrorCode::NotFound,
            Self::Conflict => PublicErrorCode::Conflict,
            Self::LimitExceeded => PublicErrorCode::LimitExceeded,
            Self::MalformedEvidence | Self::AdapterFailure => PublicErrorCode::OperationFailed,
        }
    }
}
impl fmt::Display for EvaluationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "evaluation operation failed: {:?}", self.public_code())
    }
}
impl std::error::Error for EvaluationError {}
