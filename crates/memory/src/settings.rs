//! Declarative backend selection.
//!
//! Enabled by the `settings` feature. This module owns the *shape* of memory
//! configuration; a composition binary owns the *source* — a YAML or JSON file,
//! an argument, whatever it chooses — and owns the `match` from a selected
//! backend to a constructor.
//!
//! # Why the match is not here
//!
//! A factory in this module would have to name every adapter, so the brick's
//! core would depend on all of them and the whole point of feature-gating them
//! would be lost. The binary already knows which adapters it compiled in, so it
//! is the only place that can honestly map a name to a constructor.
//!
//! # What is not yet proven
//!
//! No binary in this repository consumes these types yet, so the path from a file
//! on disk to a constructed adapter is described here but not exercised
//! end to end. That is deliberate rather than unfinished — the `match` cannot
//! live here — but it does mean the first composition binary is what turns this
//! module from a shape into a working selection mechanism.
//!
//! # Why the vocabulary is total
//!
//! [`MemoryBackend`] lists every backend the brick knows about, whether or not
//! the current build compiled it. A feature-gated enum would make a project's
//! configuration file non-portable — the same YAML would parse in one binary and
//! fail in another — and the registry validator forbids feature-gated items
//! outside an adapter module anyway. Naming a backend that was not compiled in
//! is a startup error the binary reports, not a parse error.

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::error::MemoryError;
use crate::model::{MAX_QUERY_LIMIT, Namespace};

/// Which backend a project selects.
///
/// Total and build-independent; see the module documentation.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum MemoryBackend {
    /// Deterministic in-process store. The default because it needs no
    /// configuration and makes no durability claim.
    #[default]
    InProcess,
    /// `agentic-memory`'s cognitive graph, in process.
    AgenticInProcess,
}

impl MemoryBackend {
    /// The stable wire name, matching the serde representation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::InProcess => "in_process",
            Self::AgenticInProcess => "agentic_in_process",
        }
    }

    /// Every backend the brick knows about.
    ///
    /// A composition binary's selection `match` is exhaustive over this, and
    /// documentation can be generated from it, so neither drifts from the enum.
    #[must_use]
    pub const fn all() -> [Self; 2] {
        [Self::InProcess, Self::AgenticInProcess]
    }
}

/// A project's memory configuration.
///
/// A closed schema: an unknown field is rejected rather than ignored, so a typo
/// in a configuration file fails loudly instead of silently selecting a default.
#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct MemoryConfigV1 {
    /// Closed version discriminator, so a future shape cannot be misread as this
    /// one.
    pub version: ConfigVersion,
    /// Which backend to construct.
    #[serde(default)]
    pub backend: MemoryBackend,
    /// Namespace used when a caller does not name one.
    pub default_namespace: String,
    /// Result ceiling this deployment applies, at or below [`MAX_QUERY_LIMIT`].
    ///
    /// A composition root passes this to
    /// [`crate::service::MemoryService::with_result_ceiling`]. Nothing in this
    /// module enforces it, because a configuration type that silently held an
    /// unenforced limit would be worse than having no field at all.
    #[serde(default = "default_limit")]
    pub max_query_limit: u32,
}

const fn default_limit() -> u32 {
    64
}

/// The only configuration version this build understands.
#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ConfigVersion {
    #[default]
    V1,
}

/// A configuration proven meaningful, not merely well-formed.
///
/// Deserializing [`MemoryConfigV1`] proves its shape. Converting to this type
/// proves its values: the namespace parses as an identifier and the limit is
/// within the core's ceiling. Only this type is safe to act on.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemorySettings {
    backend: MemoryBackend,
    default_namespace: Namespace,
    max_query_limit: u32,
}

impl MemorySettings {
    /// Validates a decoded configuration.
    ///
    /// # Errors
    ///
    /// [`MemoryError::InvalidId`] when the namespace is outside its grammar, and
    /// [`MemoryError::LimitExceeded`] when the limit is zero or above
    /// [`MAX_QUERY_LIMIT`].
    pub fn from_config(config: MemoryConfigV1) -> Result<Self, MemoryError> {
        let MemoryConfigV1 {
            version: ConfigVersion::V1,
            backend,
            default_namespace,
            max_query_limit,
        } = config;
        if max_query_limit == 0 || max_query_limit > MAX_QUERY_LIMIT {
            return Err(MemoryError::LimitExceeded);
        }
        Ok(Self {
            backend,
            default_namespace: Namespace::new(default_namespace)?,
            max_query_limit,
        })
    }

    #[must_use]
    pub const fn backend(&self) -> MemoryBackend {
        self.backend
    }

    #[must_use]
    pub const fn default_namespace(&self) -> &Namespace {
        &self.default_namespace
    }

    #[must_use]
    pub const fn max_query_limit(&self) -> u32 {
        self.max_query_limit
    }
}
