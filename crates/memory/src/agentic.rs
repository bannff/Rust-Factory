//! The `agentic-memory` adapter.
//!
//! Enabled by the `agentic` feature. Nothing outside this module names
//! `agentic_memory`, so the brick's default build carries none of it.
//!
//! # Mapping
//!
//! `agentic_memory::CognitiveEvent` has no tenant, namespace, key, tag, or
//! metadata field: its identity is a sequential `u64` and a `session_id`. Three
//! consequences shape this adapter.
//!
//! 1. **Isolation is structural.** One `MemoryGraph` per `(tenant, namespace)`
//!    pair, so a cross-tenant read cannot happen by a filter being wrong — there
//!    is no shared container to filter. This is stronger than a predicate over a
//!    single graph, and it is why the mapping is worth the extra bookkeeping.
//! 2. **Keys are indexed beside the graph.** A `RecordKey` maps to the assigned
//!    `u64` so replace and delete address the right node.
//! 3. **Tags and metadata are held beside the graph** rather than encoded into
//!    content, so content stays exactly what the caller wrote and a query never
//!    matches a term that only appears in an encoded header.
//!
//! # What this adapter does not use
//!
//! `agentic-memory`'s similarity, causal, centrality, decay, and belief-revision
//! engines are unused. They need a feature vector this adapter never computes —
//! there is no embedding model here — and they are graph concerns rather than
//! memory concerns. Whether they belong to the deferred `graph` family is an
//! open question and not this brick's to answer.

use std::collections::BTreeMap;
use std::sync::Mutex;

use agentic_memory::{CognitiveEventBuilder, EventType, MemoryGraph};

use crate::error::MemoryError;
use crate::model::{
    MemoryKind, MemoryQuery, MemoryRecord, Metadata, Namespace, Provenance, RecordKey, TenantId,
    WriteOutcome,
};
use crate::port::{MemoryStore, StoreGuarantees};
use crate::validation::check_capacity;

/// Feature-vector width this adapter declares to the graph.
///
/// The brick's own constant, not `agentic_memory::DEFAULT_DIMENSION`, for the
/// same reason [`crate::model::MAX_CONTENT_BYTES`] is ours: a vendor patch must
/// not change our behaviour. The value only has to satisfy the graph's insert
/// validation, because this adapter computes no embeddings — there is no model
/// here — and never reads a feature vector back.
const DIMENSION: usize = 128;

/// Fields `CognitiveEvent` cannot hold, kept beside the node it belongs to.
#[derive(Clone, Debug, Default)]
struct SideCar {
    tags: Vec<String>,
    metadata: Metadata,
    provenance: Option<Provenance>,
}

/// One `(tenant, namespace)` partition.
#[derive(Default)]
struct Partition {
    graph: Option<MemoryGraph>,
    /// Our stable key to the graph's assigned node id.
    ids: BTreeMap<String, u64>,
    /// Node id to the fields the graph cannot represent.
    sidecars: BTreeMap<u64, SideCar>,
}

impl Partition {
    fn graph_mut(&mut self) -> &mut MemoryGraph {
        self.graph
            .get_or_insert_with(|| MemoryGraph::new(DIMENSION))
    }
}

/// One tenant's namespaces.
type Tenant = BTreeMap<String, Partition>;

/// In-process `agentic-memory` store.
///
/// Named for what it guarantees. Nothing survives process exit: there is no file
/// and no `format` feature in play. A durable variant is a separate named type,
/// not a flag on this one, so a composition root cannot accidentally believe a
/// configuration field made it durable.
///
/// Nested by tenant then namespace for the same reason the local adapter is: both
/// capacity ceilings become length reads over the requesting tenant alone, rather
/// than scans across every tenant in the store.
pub struct AgenticMemoryInProcessStore {
    // One lock over every tenant. Bounded work per operation, since clause 8 caps
    // each partition and each operation touches exactly one. Poisoning is
    // permanent and process-wide; see the note on the local adapter.
    tenants: Mutex<BTreeMap<String, Tenant>>,
}

impl Default for AgenticMemoryInProcessStore {
    fn default() -> Self {
        Self::new()
    }
}

impl AgenticMemoryInProcessStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tenants: Mutex::new(BTreeMap::new()),
        }
    }
}

/// Maps our closed kind onto the vendor's closed event type.
///
/// Total in this direction on purpose: an unmapped kind would be a silent
/// behaviour change when a variant is added, so the compiler must object.
const fn event_type_of(kind: MemoryKind) -> EventType {
    match kind {
        MemoryKind::Factual => EventType::Fact,
        MemoryKind::Preference => EventType::Decision,
        MemoryKind::Procedural => EventType::Skill,
        MemoryKind::Episodic => EventType::Episode,
    }
}

/// Maps back, treating the vendor's extra variants as the nearest of ours.
///
/// `Inference` and `Correction` have no counterpart in [`MemoryKind`]. They can
/// only appear if something other than this adapter wrote to the graph, which
/// cannot happen through the port, so the fallback exists for totality rather
/// than as a lossy round trip.
const fn kind_of(event_type: EventType) -> MemoryKind {
    match event_type {
        EventType::Fact | EventType::Inference | EventType::Correction => MemoryKind::Factual,
        EventType::Decision => MemoryKind::Preference,
        EventType::Skill => MemoryKind::Procedural,
        EventType::Episode => MemoryKind::Episodic,
    }
}

impl MemoryStore for AgenticMemoryInProcessStore {
    fn put(&self, record: MemoryRecord) -> Result<WriteOutcome, MemoryError> {
        // Contract clause 6: validate at ingress. Without this the vendor's own
        // ceiling would decide what this brick accepts, which is both a different
        // limit from the core's and one that could move on a version bump.
        crate::validation::validate_record(&record)?;

        let mut tenants = self
            .tenants
            .lock()
            .map_err(|_| MemoryError::AdapterFailure)?;
        let tenant = tenants
            .entry(record.tenant_id.as_str().to_owned())
            .or_default();

        // Contract clause 8, checked before any mutation so a refusal leaves the
        // store exactly as it was.
        let existing = tenant.get(record.namespace.as_str());
        check_capacity(
            tenant.len(),
            existing.is_none(),
            existing.map_or(0, |held| held.ids.len()),
            existing.is_none_or(|held| !held.ids.contains_key(record.key.as_str())),
        )?;

        let partition = tenant
            .entry(record.namespace.as_str().to_owned())
            .or_default();

        let event = CognitiveEventBuilder::new(event_type_of(record.kind), record.content.clone())
            .created_at(
                record
                    .provenance
                    .as_ref()
                    .map_or(0, |provenance| provenance.recorded_at.as_micros()),
            )
            .build();

        // Contract clause 7: insert the replacement before removing what it
        // replaces. The reverse order means a failed insert has already destroyed
        // the previous record and left the key mapped to a node that no longer
        // exists, so the key becomes permanently unwritable. Node ids are never
        // reused, so holding both for an instant under the lock is safe.
        let id = partition
            .graph_mut()
            .add_node(event)
            .map_err(|_| MemoryError::AdapterFailure)?;

        let replaced = partition.ids.insert(record.key.as_str().to_owned(), id);
        if let Some(previous) = replaced {
            // Best effort: the replacement is already indexed, so failing to drop
            // the superseded node must not fail the write, or the index and the
            // graph would disagree. A stale node is unreachable either way,
            // because nothing but `ids` can address it.
            drop(partition.graph_mut().remove_node(previous));
            partition.sidecars.remove(&previous);
        }
        partition.sidecars.insert(
            id,
            SideCar {
                tags: record.tags,
                metadata: record.metadata,
                provenance: record.provenance,
            },
        );

        Ok(if replaced.is_some() {
            WriteOutcome::Replaced
        } else {
            WriteOutcome::Created
        })
    }

    fn get(
        &self,
        tenant_id: &TenantId,
        namespace: &Namespace,
        key: &RecordKey,
    ) -> Result<Option<MemoryRecord>, MemoryError> {
        let tenants = self
            .tenants
            .lock()
            .map_err(|_| MemoryError::AdapterFailure)?;
        // A different tenant resolves to a different subtree, so this returns None
        // without any tenant comparison being required.
        let Some(partition) = tenants
            .get(tenant_id.as_str())
            .and_then(|tenant| tenant.get(namespace.as_str()))
        else {
            return Ok(None);
        };
        let Some(&id) = partition.ids.get(key.as_str()) else {
            return Ok(None);
        };
        Ok(rebuild(partition, id, tenant_id, namespace, key))
    }

    fn query(
        &self,
        tenant_id: &TenantId,
        namespace: &Namespace,
        query: &MemoryQuery,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        // Contract clause 6: a caller holding the port directly could otherwise
        // pass `limit: u32::MAX` and bypass every ceiling in the brick.
        crate::validation::validate_query(query)?;
        let tenants = self
            .tenants
            .lock()
            .map_err(|_| MemoryError::AdapterFailure)?;
        let Some(partition) = tenants
            .get(tenant_id.as_str())
            .and_then(|tenant| tenant.get(namespace.as_str()))
        else {
            return Ok(Vec::new());
        };
        let Some(graph) = partition.graph.as_ref() else {
            return Ok(Vec::new());
        };

        // One pass over the node slice, rather than `get_node` per key.
        //
        // The vendor's `get_node` is a constant-time index lookup only while node
        // ids still equal their position. Its `remove_node` shifts the underlying
        // vector, so after the first replace or delete in a partition — which
        // happens on this adapter's ordinary write path — every `get_node`
        // degrades to a linear scan. Driving that one key at a time from a loop
        // made a bounded-looking query quadratic in partition size, and it ran
        // while holding the lock every other tenant needs.
        let mut by_id: BTreeMap<u64, &String> = BTreeMap::new();
        for (key_text, id) in &partition.ids {
            by_id.insert(*id, key_text);
        }

        let mut found = Vec::new();
        for node in graph.nodes() {
            let Some(key_text) = by_id.get(&node.id) else {
                // A node absent from the index is superseded and unreachable.
                continue;
            };
            let Ok(key) = RecordKey::new((*key_text).clone()) else {
                continue;
            };
            let sidecar = partition
                .sidecars
                .get(&node.id)
                .cloned()
                .unwrap_or_default();
            let record = assemble(node, sidecar, tenant_id, namespace, key);
            // The vendor cannot push our filters down, so the shared predicate
            // applies here. Using `MemoryQuery::matches` rather than a private
            // reimplementation is what keeps filtering identical across adapters.
            if query.matches(&record) {
                found.push(record);
            }
        }
        // Sorted rather than relying on node order, which is an insertion detail.
        // Key order is what the other adapter produces, and the two must agree.
        found.sort_by(|left, right| left.key.cmp(&right.key));
        found.truncate(query.limit as usize);
        Ok(found)
    }

    fn delete(
        &self,
        tenant_id: &TenantId,
        namespace: &Namespace,
        key: &RecordKey,
    ) -> Result<bool, MemoryError> {
        let mut tenants = self
            .tenants
            .lock()
            .map_err(|_| MemoryError::AdapterFailure)?;
        let Some(tenant) = tenants.get_mut(tenant_id.as_str()) else {
            return Ok(false);
        };
        let Some(partition) = tenant.get_mut(namespace.as_str()) else {
            return Ok(false);
        };
        let Some(&id) = partition.ids.get(key.as_str()) else {
            return Ok(false);
        };
        // Contract clause 7: drop the node first. Removing the index entry first
        // would make a failed delete report `Err` while the record had already
        // become unreachable, so the caller would be told nothing happened when in
        // fact the data was gone.
        partition
            .graph_mut()
            .remove_node(id)
            .map_err(|_| MemoryError::AdapterFailure)?;
        partition.ids.remove(key.as_str());
        partition.sidecars.remove(&id);
        // Reclaim emptied levels. Leaving a partition behind would keep a whole
        // `MemoryGraph` and its indexes alive forever, so deleting everything
        // would not return the memory and namespace churn would accumulate
        // against clause 8.
        if partition.ids.is_empty() {
            tenant.remove(namespace.as_str());
        }
        if tenant.is_empty() {
            tenants.remove(tenant_id.as_str());
        }
        Ok(true)
    }

    fn guarantees(&self) -> StoreGuarantees {
        // Honest: no file, no cross-process visibility, no crash atomicity.
        StoreGuarantees::in_process()
    }
}

/// Reassembles a record from the graph node and its sidecar.
fn rebuild(
    partition: &Partition,
    id: u64,
    tenant_id: &TenantId,
    namespace: &Namespace,
    key: &RecordKey,
) -> Option<MemoryRecord> {
    let node = partition.graph.as_ref()?.get_node(id)?;
    let sidecar = partition.sidecars.get(&id).cloned().unwrap_or_default();
    Some(assemble(node, sidecar, tenant_id, namespace, key.clone()))
}

/// Combines a graph node with its sidecar into a record.
fn assemble(
    node: &agentic_memory::CognitiveEvent,
    sidecar: SideCar,
    tenant_id: &TenantId,
    namespace: &Namespace,
    key: RecordKey,
) -> MemoryRecord {
    MemoryRecord {
        tenant_id: tenant_id.clone(),
        namespace: namespace.clone(),
        key,
        kind: kind_of(node.event_type),
        content: node.content.clone(),
        tags: sidecar.tags,
        metadata: sidecar.metadata,
        provenance: sidecar.provenance,
    }
}
