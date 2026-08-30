#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]

//! Transport-independent, token-native authorization contracts.
//!
//! This brick is the authorization substrate: a caller presents untrusted bearer
//! token bytes, and the resolver decides whether the token authorizes the
//! requested [`CapabilityV1`]. Trust comes only from cryptographic signature and
//! Datalog verification against a host-injected public root key. The verified
//! identity is an *output* of verification ([`AuthContextV1`]), never an input —
//! callers do not assert who they are, and request/correlation identifiers are
//! request scope rather than identity. Presented credentials must never enter
//! logs or errors.
//!
//! A cryptographic adapter lives in [`biscuit`], behind the `biscuit` feature,
//! and confines every `biscuit_auth` type to that module.
//!
//! This brick has no MCP surface by design. It decides what a token is
//! permitted to do, so exposing it to agents would be a privilege-escalation
//! seam: authorizing through caller input would violate the rule that caller
//! inputs never establish trusted identity, decisions, grants, or ceilings.
//!
//! The [`decision_digest`] bound into every [`AuthorizationDecisionV1::Allow`]
//! is an unkeyed in-process consistency binding of the decision to its inputs.
//! It is **not** a substitute for the adapter's cryptographic token verification
//! and does not prove provenance: a party that can construct a decision can
//! recompute its digest. Consumers must obtain decisions from a trusted injected
//! resolver and use [`verify_decision`] only to reject inconsistent values.

#[cfg(feature = "biscuit")]
pub mod biscuit;

mod error;
mod model;
mod port;
mod service;
mod validation;

pub use error::{AuthError, PublicErrorCode};
pub use model::{
    AuthContextV1, AuthorizationDecisionV1, AuthorizationRequestV1, CapabilityV1, CorrelationId,
    GrantV1, PrincipalId, RequestId, SafeDenyReasonV1, TenantId, TokenPresentation,
};
pub use port::AuthorizationResolver;
pub use service::{
    allow_decision, decision_canonical_bytes, decision_digest, deny_decision,
    grant_canonical_bytes, grant_digest, verify_decision,
};
pub use validation::{
    MAX_LOGICAL_ID_BYTES, MAX_TOKEN_PRESENTATION_BYTES, MAX_TOOL_ID_BYTES, MAX_TOOL_IDS,
    canonical_grant, validate_grant,
};
