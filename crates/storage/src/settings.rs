use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{StorageError, StorageLimits};

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum StorageConfigVersion {
    V1,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum StorageBackend {
    Local,
    Redb,
}

#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct StorageConfigV1 {
    pub version: StorageConfigVersion,
    pub backend: StorageBackend,
    pub max_objects_per_tenant: u64,
    pub max_value_bytes_per_tenant: u64,
    pub max_objects_global: u64,
    pub max_value_bytes_global: u64,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StorageSettings {
    backend: StorageBackend,
    limits: StorageLimits,
}

impl StorageSettings {
    #[must_use]
    pub const fn backend(self) -> StorageBackend {
        self.backend
    }

    #[must_use]
    pub const fn limits(self) -> StorageLimits {
        self.limits
    }
}

impl TryFrom<StorageConfigV1> for StorageSettings {
    type Error = StorageError;

    fn try_from(config: StorageConfigV1) -> Result<Self, Self::Error> {
        let StorageConfigV1 {
            version: StorageConfigVersion::V1,
            backend,
            max_objects_per_tenant,
            max_value_bytes_per_tenant,
            max_objects_global,
            max_value_bytes_global,
        } = config;
        Ok(Self {
            backend,
            limits: StorageLimits::new(
                max_objects_per_tenant,
                max_value_bytes_per_tenant,
                max_objects_global,
                max_value_bytes_global,
            )?,
        })
    }
}
