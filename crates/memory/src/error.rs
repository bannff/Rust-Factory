//! The brick's stable error taxonomy.
//!
//! One error type across the port so it stays object-safe: an adapter cannot
//! introduce an associated error, and a caller matches the same variants
//! whichever backend is configured. An adapter's own error type is mapped at
//! the module boundary and never surfaces.

use std::fmt;

/// A public error code, safe to project across a transport.
///
/// Distinct from [`MemoryError`] so an internal cause can be richer than what a
/// caller is told, without a projection accidentally leaking it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum PublicErrorCode {
    InvalidId,
    InvalidRecord,
    InvalidQuery,
    LimitExceeded,
    NotFound,
    TenantMismatch,
    Unauthorized,
    AdapterFailure,
}

impl PublicErrorCode {
    /// The stable wire name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InvalidId => "invalid_id",
            Self::InvalidRecord => "invalid_record",
            Self::InvalidQuery => "invalid_query",
            Self::LimitExceeded => "limit_exceeded",
            Self::NotFound => "not_found",
            Self::TenantMismatch => "tenant_mismatch",
            Self::Unauthorized => "unauthorized",
            Self::AdapterFailure => "adapter_failure",
        }
    }
}

/// Every way a memory operation can fail.
///
/// `Debug` is hand-written rather than derived. A derived `Debug` prints the
/// variant name, so a single `{:?}` in a log line or an error chain would
/// reintroduce exactly the distinction [`MemoryError::public_code`] exists to
/// collapse: that a key exists but belongs to another tenant.
#[derive(Clone, Copy, Eq, PartialEq)]
pub enum MemoryError {
    /// An identifier was empty, oversized, or outside its grammar.
    InvalidId,
    /// A record violated a content, tag, or metadata rule.
    InvalidRecord,
    /// A query was internally inconsistent, such as `since` at or after `until`.
    InvalidQuery,
    /// A bound in [`crate::model`] was exceeded.
    LimitExceeded,
    /// No record exists for that key in that namespace.
    ///
    /// Not produced by this brick: [`crate::port::MemoryStore::get`] reports
    /// absence as `Ok(None)` so a caller need not distinguish a missing record
    /// from a failure. This variant exists for a consumer that must turn that
    /// `None` into an error at its own boundary, and for a future adapter whose
    /// backend distinguishes the two.
    NotFound,
    /// The request's tenant does not own the addressed record.
    ///
    /// Distinct from [`Self::NotFound`] internally so a conformance test can tell
    /// a leak from an absence, but both project to `not_found` publicly so a
    /// caller cannot probe another tenant's keyspace.
    ///
    /// Also not produced by this brick, and deliberately so: isolation is
    /// structural. A foreign tenant addresses a different partition and observes
    /// absence, so there is no point at which a mismatch is detected and could be
    /// reported. This variant is the vocabulary for an adapter whose backend
    /// cannot partition and must compare tenants explicitly.
    TenantMismatch,
    /// The caller is not permitted to perform the operation.
    ///
    /// Not produced by the core today, which performs no authorization.
    ///
    /// It lives here rather than in the adapter for two reasons. First, this
    /// crate's [`PublicErrorCode`] is its single wire vocabulary and
    /// `public_code` its single projection point; an adapter-local error enum
    /// would give one crate two taxonomies over the same operations. Second, a
    /// `MemoryStore` fronting a remote or access-controlled backend is a concrete
    /// future producer — unlike a control-plane concern that could never arise
    /// below the port.
    ///
    /// It is a distinct variant, not folded into [`Self::AdapterFailure`], because
    /// a caller must be able to tell a **permanent** refusal from a **transient**
    /// fault. Collapsing them makes an agent retry-loop forever on a capability it
    /// will never hold.
    ///
    /// It deliberately carries no reason. Denied, not-enabled, and a tampered
    /// decision are indistinguishable, so the variant cannot be used to probe
    /// which capabilities exist.
    Unauthorized,
    /// The backend failed for a reason the caller cannot act on.
    AdapterFailure,
}

impl MemoryError {
    /// The code a caller may be shown.
    ///
    /// [`Self::TenantMismatch`] deliberately projects to `not_found`: telling a
    /// caller a key exists but belongs to someone else confirms its existence.
    #[must_use]
    pub const fn public_code(self) -> PublicErrorCode {
        match self {
            Self::InvalidId => PublicErrorCode::InvalidId,
            Self::InvalidRecord => PublicErrorCode::InvalidRecord,
            Self::InvalidQuery => PublicErrorCode::InvalidQuery,
            Self::LimitExceeded => PublicErrorCode::LimitExceeded,
            Self::NotFound | Self::TenantMismatch => PublicErrorCode::NotFound,
            Self::Unauthorized => PublicErrorCode::Unauthorized,
            Self::AdapterFailure => PublicErrorCode::AdapterFailure,
        }
    }
}

impl fmt::Display for MemoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.public_code().as_str())
    }
}

impl fmt::Debug for MemoryError {
    /// Shows only the public code, so `{:?}` is as safe as `{}`.
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.public_code().as_str())
    }
}

impl std::error::Error for MemoryError {}
