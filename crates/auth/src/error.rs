use std::fmt;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicErrorCode {
    InvalidId,
    InvalidGrant,
    InvalidToken,
    LimitExceeded,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthError {
    InvalidId,
    InvalidGrant,
    InvalidToken,
    LimitExceeded,
}

impl AuthError {
    #[must_use]
    pub const fn public_code(self) -> PublicErrorCode {
        match self {
            Self::InvalidId => PublicErrorCode::InvalidId,
            Self::InvalidGrant => PublicErrorCode::InvalidGrant,
            Self::InvalidToken => PublicErrorCode::InvalidToken,
            Self::LimitExceeded => PublicErrorCode::LimitExceeded,
        }
    }
}

impl fmt::Display for AuthError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "auth operation failed: {:?}", self.public_code())
    }
}

impl std::error::Error for AuthError {}
