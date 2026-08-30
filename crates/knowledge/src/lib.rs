#![forbid(unsafe_code)]
//! Bounded, deterministic knowledge retrieval contracts.
//!
//! Rust Factory metadata: `family = "knowledge"`, `role = "brick"`,
//! `status = "implemented"`.
//!
//! The framework-free core defines validated request and document values, an
//! object-safe index seam, and a service that validates complete adapter output
//! before exposing consumer-facing results.

mod error;
mod model;
mod port;
mod service;
mod validation;

#[cfg(feature = "static")]
pub mod r#static;

pub use error::KnowledgeError;
pub use model::{
    DocumentId, KnowledgeDocument, KnowledgeHit, NamespaceId, PrincipalId, Query, SearchContext,
    SearchLimit, SearchRequest, SearchResult, TenantId,
};
pub use port::KnowledgeIndex;
pub use service::KnowledgeService;
pub use validation::{
    MAX_DOCUMENT_TEXT_BYTES, MAX_IDENTIFIER_BYTES, MAX_QUERY_BYTES, MAX_RESULT_TEXT_BYTES,
    MAX_SEARCH_LIMIT, MAX_STATIC_DOCUMENTS, MAX_STATIC_TEXT_BYTES,
};
