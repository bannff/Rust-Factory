use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicErrorCode {
    InvalidId,
    InvalidEvent,
    InvalidQuery,
    InvalidConfiguration,
    InvalidTraceContext,
    InvalidSpan,
    InvalidMetric,
    LimitExceeded,
    OperationFailed,
}

#[derive(Clone, Copy, Eq, PartialEq)]
pub enum ObservabilityError {
    InvalidId,
    InvalidEvent,
    InvalidQuery,
    InvalidConfiguration,
    InvalidTraceContext,
    InvalidSpan,
    InvalidMetric,
    LimitExceeded,
    AdapterFailure,
}
impl ObservabilityError {
    #[must_use]
    pub const fn public_code(self) -> PublicErrorCode {
        match self {
            Self::InvalidId => PublicErrorCode::InvalidId,
            Self::InvalidEvent => PublicErrorCode::InvalidEvent,
            Self::InvalidQuery => PublicErrorCode::InvalidQuery,
            Self::InvalidConfiguration => PublicErrorCode::InvalidConfiguration,
            Self::InvalidTraceContext => PublicErrorCode::InvalidTraceContext,
            Self::InvalidSpan => PublicErrorCode::InvalidSpan,
            Self::InvalidMetric => PublicErrorCode::InvalidMetric,
            Self::LimitExceeded => PublicErrorCode::LimitExceeded,
            Self::AdapterFailure => PublicErrorCode::OperationFailed,
        }
    }
}
impl fmt::Debug for ObservabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.public_code().fmt(formatter)
    }
}
impl fmt::Display for ObservabilityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "observability operation failed: {:?}",
            self.public_code()
        )
    }
}
impl std::error::Error for ObservabilityError {}
