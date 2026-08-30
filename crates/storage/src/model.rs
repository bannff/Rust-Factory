use std::fmt;
use std::num::NonZeroU64;

use crate::error::StorageError;
use crate::validation;

pub const MAX_TENANT_ID_BYTES: u16 = 128;
pub const MAX_NAMESPACE_BYTES: u16 = 128;
pub const MAX_OBJECT_KEY_BYTES: u16 = 1_024;
pub const MAX_OBJECT_VALUE_BYTES: u32 = 1_048_576;
pub const MAX_LIST_LIMIT: u32 = 1_000;
pub const MAX_OBJECTS_PER_TENANT: u64 = 100_000;
pub const MAX_VALUE_BYTES_PER_TENANT: u64 = 1_073_741_824;
pub const MAX_OBJECTS_GLOBAL: u64 = 1_000_000;
pub const MAX_VALUE_BYTES_GLOBAL: u64 = 8_589_934_592;

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct TenantId(String);

impl TenantId {
    pub fn new(value: impl Into<String>) -> Result<Self, StorageError> {
        let value = value.into();
        validation::validate_tenant_id(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct Namespace(String);

impl Namespace {
    pub fn new(value: impl Into<String>) -> Result<Self, StorageError> {
        let value = value.into();
        validation::validate_namespace(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub struct ObjectKey(Vec<u8>);

impl ObjectKey {
    pub fn new(value: impl Into<Vec<u8>>) -> Result<Self, StorageError> {
        let value = value.into();
        validation::validate_key(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[allow(dead_code)]
    pub(crate) fn minimum() -> Self {
        Self(vec![0])
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectValue(Vec<u8>);

impl ObjectValue {
    pub fn new(value: impl Into<Vec<u8>>) -> Result<Self, StorageError> {
        let value = value.into();
        validation::validate_value(&value)?;
        Ok(Self(value))
    }

    #[must_use]
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    #[must_use]
    pub fn into_bytes(self) -> Vec<u8> {
        self.0
    }
}

#[derive(Clone, Eq, PartialEq)]
pub struct ObjectVersion(NonZeroU64);

impl ObjectVersion {
    #[allow(
        dead_code,
        reason = "constructed by adapters, so default and settings-only builds do not use it"
    )]
    pub(crate) fn from_revision(revision: u64) -> Result<Self, StorageError> {
        NonZeroU64::new(revision)
            .map(Self)
            .ok_or(StorageError::CorruptStore)
    }

    #[allow(dead_code)]
    pub(crate) const fn revision(&self) -> u64 {
        self.0.get()
    }
}

impl fmt::Debug for ObjectVersion {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("ObjectVersion(<opaque>)")
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StorageScope {
    pub tenant_id: TenantId,
    pub namespace: Namespace,
}

impl StorageScope {
    #[must_use]
    pub const fn new(tenant_id: TenantId, namespace: Namespace) -> Self {
        Self {
            tenant_id,
            namespace,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ListLimit(u32);

impl ListLimit {
    pub fn new(value: u32) -> Result<Self, StorageError> {
        if value == 0 || value > MAX_LIST_LIMIT {
            return Err(StorageError::InvalidListLimit);
        }
        Ok(Self(value))
    }

    #[must_use]
    pub const fn get(self) -> u32 {
        self.0
    }

    #[allow(dead_code)]
    pub(crate) fn as_usize(self) -> Result<usize, StorageError> {
        usize::try_from(self.0).map_err(|_| StorageError::OperationFailed)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListRequest {
    pub after_key: Option<ObjectKey>,
    pub limit: ListLimit,
}

impl ListRequest {
    #[must_use]
    pub const fn new(after_key: Option<ObjectKey>, limit: ListLimit) -> Self {
        Self { after_key, limit }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PutCondition {
    Any,
    IfAbsent,
    IfVersion(ObjectVersion),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DeleteCondition {
    Any,
    IfVersion(ObjectVersion),
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PutOutcome {
    Created { version: ObjectVersion },
    Replaced { version: ObjectVersion },
    Conflict,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DeleteOutcome {
    Deleted,
    NotFound,
    Conflict,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct StoredObject {
    pub version: ObjectVersion,
    pub value: ObjectValue,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObjectMetadata {
    pub key: ObjectKey,
    pub version: ObjectVersion,
    pub size_bytes: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ListPage {
    pub objects: Vec<ObjectMetadata>,
    pub has_more: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(
    clippy::struct_field_names,
    reason = "limit field names are normative public contract terminology"
)]
pub struct StorageLimits {
    max_objects_per_tenant: u64,
    max_value_bytes_per_tenant: u64,
    max_objects_global: u64,
    max_value_bytes_global: u64,
}

impl StorageLimits {
    pub fn new(
        max_objects_per_tenant: u64,
        max_value_bytes_per_tenant: u64,
        max_objects_global: u64,
        max_value_bytes_global: u64,
    ) -> Result<Self, StorageError> {
        validation::validate_limits(
            max_objects_per_tenant,
            max_value_bytes_per_tenant,
            max_objects_global,
            max_value_bytes_global,
        )?;
        Ok(Self {
            max_objects_per_tenant,
            max_value_bytes_per_tenant,
            max_objects_global,
            max_value_bytes_global,
        })
    }

    #[must_use]
    pub const fn max_objects_per_tenant(self) -> u64 {
        self.max_objects_per_tenant
    }

    #[must_use]
    pub const fn max_value_bytes_per_tenant(self) -> u64 {
        self.max_value_bytes_per_tenant
    }

    #[must_use]
    pub const fn max_objects_global(self) -> u64 {
        self.max_objects_global
    }

    #[must_use]
    pub const fn max_value_bytes_global(self) -> u64 {
        self.max_value_bytes_global
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PersistenceGuarantee {
    Volatile,
    CleanRestart,
    ImmediateCommit,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[allow(
    clippy::struct_excessive_bools,
    reason = "independent guarantee flags are a normative public contract"
)]
pub struct StoreGuarantees {
    pub persistence: PersistenceGuarantee,
    pub shared_across_processes: bool,
    pub per_operation_atomic: bool,
    pub conditional_writes: bool,
    pub eviction: bool,
    pub limits: StorageLimits,
}
