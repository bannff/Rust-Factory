//! Typed memory records and validated identifiers.
//!
//! Every type here is framework-neutral. An adapter converts to and from its
//! own representation; none of its types appear in this module.

use std::collections::BTreeMap;
use std::fmt;

use crate::error::MemoryError;
use crate::validation::is_logical_id;

/// Maximum bytes of a single record's content.
///
/// Deliberately the core's own constant rather than an adapter's. Inheriting a
/// vendor limit would let a patch release change this crate's public contract.
pub const MAX_CONTENT_BYTES: usize = 32 * 1024;
/// Maximum bytes of one metadata key or value.
pub const MAX_METADATA_ENTRY_BYTES: usize = 1024;
/// Maximum number of metadata entries on one record.
pub const MAX_METADATA_ENTRIES: usize = 32;
/// Maximum bytes of an identifier.
pub const MAX_ID_BYTES: usize = 128;
/// Maximum number of tags on one record.
pub const MAX_TAGS: usize = 16;
/// Maximum records one query may return.
///
/// A deployment may narrow this through
/// [`crate::service::MemoryService::with_result_ceiling`] but never widen it.
pub const MAX_QUERY_LIMIT: u32 = 256;

/// Maximum records one `(tenant, namespace)` partition may hold.
///
/// Every other bound in this module is per request, which bounds the cost of one
/// call and nothing else. Without a capacity ceiling a caller making entirely
/// valid requests can grow the store without limit.
///
/// # What this does and does not bound
///
/// Together with [`MAX_TENANT_NAMESPACES`] this bounds one tenant to at most
/// `4_096 * 64` records. It does **not** bound the number of tenants, so the
/// honest claim is per-tenant containment: one tenant cannot exhaust the host on
/// its own, and cannot grow without limit. A deployment admitting unbounded
/// tenants needs its own admission control, which is not this brick's concern.
pub const MAX_PARTITION_RECORDS: usize = 4_096;

/// Maximum distinct namespaces one tenant may occupy.
///
/// The namespace is caller-supplied, so without this a caller could mint
/// unbounded partitions and defeat [`MAX_PARTITION_RECORDS`] by spreading across
/// them rather than filling one.
pub const MAX_TENANT_NAMESPACES: usize = 64;

/// The largest a single accepted record can be, counting every bounded field.
///
/// Stated explicitly because the per-field limits above do not add up to an
/// obvious total: content plus the maximum metadata payload is roughly three
/// times [`MAX_CONTENT_BYTES`]. A caller sizing a buffer or a transport ceiling
/// needs this number rather than the content limit alone.
pub const MAX_RECORD_BYTES: usize = MAX_CONTENT_BYTES
    + MAX_METADATA_ENTRIES * 2 * MAX_METADATA_ENTRY_BYTES
    + MAX_TAGS * MAX_ID_BYTES;
/// Maximum bytes of a query's text term.
pub const MAX_TERM_BYTES: usize = 1024;

macro_rules! logical_id {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
        pub struct $name(String);

        impl $name {
            /// Constructs the identifier, rejecting any value outside the
            /// grammar.
            ///
            /// # Errors
            ///
            /// Returns [`MemoryError::InvalidId`] when the value is empty,
            /// longer than [`MAX_ID_BYTES`], or contains a character outside
            /// lowercase ASCII alphanumerics, `_`, `-`, and `.`.
            pub fn new(value: impl Into<String>) -> Result<Self, MemoryError> {
                let value = value.into();
                is_logical_id(&value)
                    .then_some(Self(value))
                    .ok_or(MemoryError::InvalidId)
            }

            #[must_use]
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(&self.0)
            }
        }
    };
}

logical_id!(
    TenantId,
    "Owning tenant. Isolation is scoped by this value."
);
logical_id!(
    Namespace,
    "A partition within a tenant, so unrelated concerns do not share a recall surface."
);
logical_id!(
    RecordKey,
    "Stable identity of one record within a namespace."
);
logical_id!(RunId, "Identifies the run that produced a record.");

/// What kind of thing a record is.
///
/// Closed on purpose: an open string would let each adapter invent its own
/// vocabulary, and a caller could not then filter portably. Adding a variant is
/// a deliberate contract change.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub enum MemoryKind {
    /// Something observed to be true.
    Factual,
    /// A choice to honour in future work.
    Preference,
    /// How to carry out a task.
    Procedural,
    /// Something that happened.
    Episodic,
}

impl MemoryKind {
    /// The stable wire name. Never derive this from the variant name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Factual => "factual",
            Self::Preference => "preference",
            Self::Procedural => "procedural",
            Self::Episodic => "episodic",
        }
    }

    /// Every variant, for exhaustive iteration at a boundary.
    #[must_use]
    pub const fn all() -> [Self; 4] {
        [
            Self::Factual,
            Self::Preference,
            Self::Procedural,
            Self::Episodic,
        ]
    }
}

/// A microsecond timestamp.
///
/// The core never reads a clock. A value arrives through the injected
/// [`crate::port::Clock`] so a test can be deterministic and an adapter cannot
/// silently disagree with the service about what time it is.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd, Hash)]
pub struct Timestamp(u64);

impl Timestamp {
    #[must_use]
    pub const fn from_micros(micros: u64) -> Self {
        Self(micros)
    }

    #[must_use]
    pub const fn as_micros(self) -> u64 {
        self.0
    }
}

/// Where a record came from.
///
/// Optional because a record may be authored directly, but when present it ties
/// a memory to the run that produced it, which is what makes a learning loop
/// auditable.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Provenance {
    pub run_id: RunId,
    pub recorded_at: Timestamp,
}

impl Provenance {
    /// Builds provenance from a run identifier and a microsecond timestamp.
    ///
    /// For a composition root rehydrating records from elsewhere. On the ordinary
    /// write path [`crate::service::MemoryService`] stamps provenance itself from
    /// the injected clock, so a caller cannot backdate a memory.
    ///
    /// # Errors
    ///
    /// [`MemoryError::InvalidId`] when `run_id` is outside its grammar.
    pub fn new(run_id: &str, recorded_at_micros: u64) -> Result<Self, MemoryError> {
        Ok(Self {
            run_id: RunId::new(run_id)?,
            recorded_at: Timestamp::from_micros(recorded_at_micros),
        })
    }
}

/// Opaque per-record metadata.
///
/// A `BTreeMap` rather than a typed struct because the useful keys differ per
/// consumer, and ordered so canonical bytes are stable. Bounded by
/// [`MAX_METADATA_ENTRIES`] and [`MAX_METADATA_ENTRY_BYTES`]; the core attaches
/// no meaning to any key.
pub type Metadata = BTreeMap<String, String>;

/// One memory record.
///
/// # Validity is a checkpoint, not a type-level property
///
/// The fields are public, so [`MemoryRecord::validated`] proves a record was
/// valid *at the moment it was called* and nothing more — a caller can mutate a
/// field afterwards. Do not read the existence of `validated` as a guarantee that
/// any `MemoryRecord` in hand is well-formed.
///
/// What actually holds the line is contract clause 6 of
/// [`crate::port::MemoryStore`]: every adapter revalidates at ingress, so an
/// invalid record cannot be stored however it was constructed. The fields stay
/// public because the alternative — private fields with eight accessors and a
/// builder — buys no safety once ingress validation exists, and makes the type
/// tedious to pattern match in exactly the code that most needs to.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MemoryRecord {
    pub tenant_id: TenantId,
    pub namespace: Namespace,
    pub key: RecordKey,
    pub kind: MemoryKind,
    pub content: String,
    pub tags: Vec<String>,
    pub metadata: Metadata,
    pub provenance: Option<Provenance>,
}

impl MemoryRecord {
    /// Checks a constructed record against every rule in [`crate::validation`].
    ///
    /// Consumes and returns the record so the check reads as part of construction
    /// rather than as a statement a caller might forget to act on. See the type
    /// documentation for why this is a checkpoint and not a guarantee.
    ///
    /// # Errors
    ///
    /// Returns a [`MemoryError`] when a bound in this module is exceeded or a tag
    /// is outside its grammar.
    pub fn validated(self) -> Result<Self, MemoryError> {
        crate::validation::validate_record(&self)?;
        Ok(self)
    }
}

/// A closed set of filters over one namespace.
///
/// Closed rather than an open query language so every adapter can honestly
/// implement all of it. A capability an adapter cannot serve does not belong
/// here — it belongs to a separate port that only capable adapters implement,
/// so a caller discovers the gap at composition time rather than at runtime.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub struct MemoryQuery {
    /// Restrict to these kinds. Empty means every kind.
    pub kinds: Vec<MemoryKind>,
    /// Require every one of these tags.
    pub tags: Vec<String>,
    /// Substring match against content. `None` means no content filter.
    pub term: Option<String>,
    /// Inclusive lower bound on provenance time.
    pub since: Option<Timestamp>,
    /// Exclusive upper bound on provenance time.
    pub until: Option<Timestamp>,
    /// Maximum records to return, capped by [`MAX_QUERY_LIMIT`].
    pub limit: u32,
}

impl MemoryQuery {
    /// A query returning up to `limit` records with no other filter.
    ///
    /// # Errors
    ///
    /// Returns [`MemoryError::LimitExceeded`] when `limit` is zero or above
    /// [`MAX_QUERY_LIMIT`].
    pub fn all(limit: u32) -> Result<Self, MemoryError> {
        let query = Self {
            limit,
            ..Self::default()
        };
        crate::validation::validate_query(&query)?;
        Ok(query)
    }

    /// Reports whether a record satisfies every filter.
    ///
    /// Adapters that cannot push a filter down to storage use this so filtering
    /// is identical everywhere, rather than each adapter reimplementing it.
    #[must_use]
    pub fn matches(&self, record: &MemoryRecord) -> bool {
        if !self.kinds.is_empty() && !self.kinds.contains(&record.kind) {
            return false;
        }
        if !self
            .tags
            .iter()
            .all(|tag| record.tags.iter().any(|held| held == tag))
        {
            return false;
        }
        if let Some(term) = &self.term
            && !record.content.contains(term.as_str())
        {
            return false;
        }
        match (self.since, self.until, record.provenance.as_ref()) {
            (None, None, _) => true,
            (_, _, None) => false,
            (since, until, Some(provenance)) => {
                since.is_none_or(|bound| provenance.recorded_at >= bound)
                    && until.is_none_or(|bound| provenance.recorded_at < bound)
            }
        }
    }
}

/// Whether a write created a record or replaced one.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WriteOutcome {
    Created,
    Replaced,
}
