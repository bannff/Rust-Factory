#![forbid(unsafe_code)]
#![deny(rust_2018_idioms)]
#![allow(clippy::missing_errors_doc)]
#![allow(clippy::needless_pass_by_value)]
//! Evaluation capability brick (`family = "evaluation"`, `role = "brick"`,
//! `status = "implemented"`) for framework-neutral terminal evidence assessment.
//!
//! The crate owns no runtime or process lifecycle. Executor futures are polled and
//! cancelled by callers; dropping one provides no cross-process cancellation or
//! recovery guarantee.

pub mod canonical;
pub mod error;
#[cfg(feature = "local")]
pub mod local;
#[cfg(feature = "mcp")]
pub mod mcp;
#[cfg(feature = "memory")]
pub mod memory;
pub mod model;
pub mod port;
#[cfg(feature = "serdes-ai-evals")]
pub mod serdes_ai_evals;
pub mod service;
#[cfg(feature = "settings")]
pub mod settings;
pub mod validation;

pub use canonical::{
    definition_canonical_bytes, definition_digest, result_canonical_bytes, result_digest,
    snapshot_canonical_bytes, snapshot_digest,
};
pub use error::{EvaluationError, PublicErrorCode};
pub use model::*;
pub use port::{EvaluationExecutor, EvaluationFuture, EvaluationStore, WorkflowEvidenceReader};
pub use service::{EvaluationService, evaluate, evaluate_and_store};
pub use validation::{
    validate_assessment, validate_definition, validate_logical_key, validate_result,
    validate_snapshot,
};
