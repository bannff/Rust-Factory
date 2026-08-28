#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
//! Tenant-scoped agent memory with a selectable backend.
//!
//! # Shape
//!
//! [`port::MemoryStore`] is the contract; [`service::MemoryService`] validates
//! and scopes every request before it reaches an implementation. No feature is
//! enabled by default, so the default build is the framework-free capability and
//! every adapter — including the standard-library-only one — is opt-in.
//!
//! | Feature | Module | Brings in |
//! |---|---|---|
//! | `local` | `local` | nothing beyond `std` |
//! | `agentic` | `agentic` | `agentic-memory`'s cognitive graph |
//! | `settings` | `settings` | `serde` and `schemars` for declarative selection |
//!
//! # Selecting a backend
//!
//! `settings::MemoryConfigV1` deserializes a project's choice and
//! `settings::MemorySettings` proves it meaningful. The `match` from a chosen
//! backend to a constructor belongs to the composition binary, which is the only
//! place that knows which adapters were compiled in. A factory here would force
//! this crate to depend on every adapter and defeat the feature gating.
//!
//! # Boundary discipline
//!
//! `serde` and `schemars` appear only in `settings`. A boundary DTO converts into
//! a [`model`] type and is never used as a domain type itself, because shape and
//! validity are separate concerns and decoding proves only the first.
//!
//! Identifiers ([`TenantId`], [`Namespace`], [`RecordKey`], [`RunId`]) do wrap a
//! private field behind a fallible constructor, so an invalid one cannot exist.
//! Aggregates ([`MemoryRecord`], [`MemoryQuery`]) do not: their fields are public
//! and validation is a checkpoint rather than a type-level property. What holds
//! the line for those is contract clause 6 of [`port::MemoryStore`] — every
//! adapter revalidates at ingress, so an invalid aggregate cannot be stored
//! however it was built.

pub mod error;
pub mod model;
pub mod port;
pub mod service;
pub mod validation;

#[cfg(feature = "agentic")]
pub mod agentic;
#[cfg(feature = "local")]
pub mod local;
#[cfg(feature = "settings")]
pub mod settings;

pub use error::{MemoryError, PublicErrorCode};
pub use model::{
    MAX_CONTENT_BYTES, MAX_ID_BYTES, MAX_METADATA_ENTRIES, MAX_METADATA_ENTRY_BYTES,
    MAX_PARTITION_RECORDS, MAX_QUERY_LIMIT, MAX_RECORD_BYTES, MAX_TAGS, MAX_TENANT_NAMESPACES,
    MAX_TERM_BYTES, MemoryKind, MemoryQuery, MemoryRecord, Metadata, Namespace, Provenance,
    RecordKey, RunId, TenantId, Timestamp, WriteOutcome,
};
pub use port::{Clock, MemoryStore, StoreGuarantees};
pub use service::{MemoryContext, MemoryService, RememberRequest};
