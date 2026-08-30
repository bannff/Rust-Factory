use std::fmt;

/// Closed, redacted Storage error taxonomy.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum StorageError {
    InvalidTenantId,
    InvalidNamespace,
    InvalidObjectKey,
    InvalidValue,
    InvalidListLimit,
    InvalidLimits,
    LimitExceeded,
    RevisionExhausted,
    LockUnavailable,
    CorruptStore,
    OperationFailed,
}

impl StorageError {
    /// Stable public code safe for logs and boundary projection.
    #[must_use]
    pub const fn code(self) -> &'static str {
        match self {
            Self::InvalidTenantId => "invalid_tenant_id",
            Self::InvalidNamespace => "invalid_namespace",
            Self::InvalidObjectKey => "invalid_object_key",
            Self::InvalidValue => "invalid_value",
            Self::InvalidListLimit => "invalid_list_limit",
            Self::InvalidLimits => "invalid_limits",
            Self::LimitExceeded => "limit_exceeded",
            Self::RevisionExhausted => "revision_exhausted",
            Self::LockUnavailable => "lock_unavailable",
            Self::CorruptStore => "corrupt_store",
            Self::OperationFailed => "operation_failed",
        }
    }
}

impl fmt::Display for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl fmt::Debug for StorageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.code())
    }
}

impl std::error::Error for StorageError {}
