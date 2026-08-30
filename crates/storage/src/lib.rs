#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]
//! Storage capability brick (`family = "storage"`, `role = "brick"`,
//! `status = "implemented"`).
//!
//! The core is synchronous and transport-independent. It stores bounded opaque
//! versioned objects; callers retain serialization, domain transitions, retries,
//! audit, and multi-object transaction ownership.

pub mod error;
#[cfg(feature = "local")]
pub mod local;
pub mod model;
pub mod port;
#[cfg(feature = "redb")]
pub mod redb;
mod service;
#[cfg(feature = "settings")]
pub mod settings;
pub mod validation;

pub use error::StorageError;
pub use model::{
    DeleteCondition, DeleteOutcome, ListLimit, ListPage, ListRequest, MAX_LIST_LIMIT,
    MAX_NAMESPACE_BYTES, MAX_OBJECT_KEY_BYTES, MAX_OBJECT_VALUE_BYTES, MAX_OBJECTS_GLOBAL,
    MAX_OBJECTS_PER_TENANT, MAX_TENANT_ID_BYTES, MAX_VALUE_BYTES_GLOBAL,
    MAX_VALUE_BYTES_PER_TENANT, Namespace, ObjectKey, ObjectMetadata, ObjectValue, ObjectVersion,
    PersistenceGuarantee, PutCondition, PutOutcome, StorageLimits, StorageScope, StoreGuarantees,
    StoredObject, TenantId,
};
pub use port::ObjectStore;
