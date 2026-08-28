//! The consumed ports every adapter implements.
//!
//! # Object safety is deliberate
//!
//! [`MemoryStore`] is object-safe: `&self` receivers, no generic methods, no
//! `impl Trait`, one concrete error type. A composition binary must be able to
//! read a configuration file, pick a backend, and hold the result as
//! `Arc<dyn MemoryStore>`. A generic-only port would force the choice into the
//! type system, which is exactly what declarative selection cannot do.
//!
//! # What is not here
//!
//! Graph traversal, similarity, centrality, and decay are absent on purpose.
//! Only a graph-capable backend could implement them, so including them would
//! make this port unimplementable by a plain key-value or SQL backend and the
//! brick would stop being polymorphic. A capability that not every adapter can
//! serve belongs to its own port, so a caller that needs it fails to compose
//! rather than failing at request time.

use std::sync::Arc;

use crate::error::MemoryError;
use crate::model::{
    MemoryQuery, MemoryRecord, Namespace, RecordKey, TenantId, Timestamp, WriteOutcome,
};

/// Supplies the current time.
///
/// Injected so the core never reads a clock: a test is then deterministic, and
/// an adapter cannot disagree with the service about what time it is.
pub trait Clock: Send + Sync {
    fn now(&self) -> Timestamp;
}

/// A tenant-scoped view of storage.
///
/// # Contract
///
/// An implementation must honour all of the following. The suite in
/// `tests/adapter_contract.rs` checks each one, and an adapter is not conformant
/// until it passes.
///
/// 1. **Tenant isolation.** No operation may observe or modify a record whose
///    tenant differs from the request's, and a cross-tenant read reports absence
///    rather than a distinguishable refusal.
/// 2. **Namespace isolation.** A key is unique within a namespace, not across
///    namespaces; the same key in two namespaces is two records.
/// 3. **Idempotent replace.** Writing an existing key replaces that record and
///    reports [`WriteOutcome::Replaced`]; it never duplicates.
/// 4. **Query totality.** Every filter in [`MemoryQuery`] is honoured. An
///    adapter that cannot push a filter down must apply
///    [`MemoryQuery::matches`] itself rather than ignore it.
/// 5. **Bounded results.** At most `limit` records are returned.
/// 6. **Validation at ingress.** An adapter must itself reject a record that
///    fails [`crate::validation::validate_record`] and a query that fails
///    [`crate::validation::validate_query`], returning the same [`MemoryError`]
///    any other adapter would.
///
///    This is not redundant with [`crate::service::MemoryService`]. `MemoryStore`
///    is public and the fields of [`MemoryRecord`] and [`MemoryQuery`] are
///    public, so a composition root or a peer brick can hold an adapter directly
///    and hand it anything — including `limit: u32::MAX`, which would otherwise
///    bypass every result ceiling in the brick. If each adapter instead deferred
///    to its backend's own limits, two adapters would disagree on identical input
///    and a backend's version bump could change this brick's behaviour.
/// 7. **Failure leaves no trace.** An operation that returns `Err` must not have
///    applied a partial effect. A write that fails must leave any previous
///    record for that key intact and the key writable.
/// 8. **Bounded capacity.** A write that would exceed
///    [`crate::model::MAX_PARTITION_RECORDS`] for its partition, or
///    [`crate::model::MAX_TENANT_NAMESPACES`] for its tenant, must be refused
///    with [`MemoryError::LimitExceeded`]. Replacing an existing key consumes no
///    additional capacity and must always be permitted.
///
///    Per-request limits bound the cost of one call and nothing else. Without a
///    capacity ceiling a caller issuing only valid requests can exhaust the host,
///    and in a shared process every other tenant pays for it.
///
/// # Read-your-writes is a precondition, not a guarantee to declare
///
/// Clauses 1 through 3 assume that a completed [`MemoryStore::put`] is observable
/// by a subsequent [`MemoryStore::get`] on the same handle. An eventually
/// consistent backend — a vector store, or a SQL store fronted by read replicas —
/// cannot satisfy that, and must not implement this port by relaxing the clauses
/// quietly.
///
/// This is stated as a precondition rather than added to [`StoreGuarantees`] on
/// purpose. A guarantee flag would let such a backend pass composition while
/// every caller written against clause 3 silently broke, and "sometimes your
/// write is not there yet" changes how a caller must be written, not merely what
/// it should expect. A backend that cannot promise it needs a port whose contract
/// admits staleness.
///
/// # Cost and contention
///
/// An implementation should keep the work of one operation proportional to the
/// requesting tenant's own partition, never to the whole store. A backend whose
/// per-key lookup degrades as the store grows must not be driven one key at a
/// time inside a query loop, or a caller can turn a bounded-looking query into
/// quadratic work. Where a single lock covers several tenants, that cost is also
/// latency every other tenant waits for.
pub trait MemoryStore: Send + Sync {
    /// Creates or replaces a record.
    ///
    /// # Errors
    ///
    /// [`MemoryError::AdapterFailure`] when the backend cannot complete.
    fn put(&self, record: MemoryRecord) -> Result<WriteOutcome, MemoryError>;

    /// Reads one record by key.
    ///
    /// # Errors
    ///
    /// [`MemoryError::AdapterFailure`] when the backend cannot complete.
    /// Absence is `Ok(None)`, not an error, so a caller need not distinguish a
    /// missing record from a failure.
    fn get(
        &self,
        tenant_id: &TenantId,
        namespace: &Namespace,
        key: &RecordKey,
    ) -> Result<Option<MemoryRecord>, MemoryError>;

    /// Returns every record in one namespace matching the query.
    ///
    /// # Errors
    ///
    /// [`MemoryError::AdapterFailure`] when the backend cannot complete.
    fn query(
        &self,
        tenant_id: &TenantId,
        namespace: &Namespace,
        query: &MemoryQuery,
    ) -> Result<Vec<MemoryRecord>, MemoryError>;

    /// Removes one record, reporting whether it existed.
    ///
    /// # Errors
    ///
    /// [`MemoryError::AdapterFailure`] when the backend cannot complete.
    fn delete(
        &self,
        tenant_id: &TenantId,
        namespace: &Namespace,
        key: &RecordKey,
    ) -> Result<bool, MemoryError>;

    /// What this backend actually guarantees.
    ///
    /// Required rather than optional so an adapter cannot stay silent about
    /// durability. A caller may surface it; a specification may assert on it.
    fn guarantees(&self) -> StoreGuarantees;
}

// Blanket impls so a shared handle satisfies the port. Without these, a binary
// holding `Arc<Concrete>` could not pass it where `&dyn MemoryStore` is wanted,
// and every call site would wrap by hand.
impl<T: MemoryStore + ?Sized> MemoryStore for Arc<T> {
    fn put(&self, record: MemoryRecord) -> Result<WriteOutcome, MemoryError> {
        (**self).put(record)
    }

    fn get(
        &self,
        tenant_id: &TenantId,
        namespace: &Namespace,
        key: &RecordKey,
    ) -> Result<Option<MemoryRecord>, MemoryError> {
        (**self).get(tenant_id, namespace, key)
    }

    fn query(
        &self,
        tenant_id: &TenantId,
        namespace: &Namespace,
        query: &MemoryQuery,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        (**self).query(tenant_id, namespace, query)
    }

    fn delete(
        &self,
        tenant_id: &TenantId,
        namespace: &Namespace,
        key: &RecordKey,
    ) -> Result<bool, MemoryError> {
        (**self).delete(tenant_id, namespace, key)
    }

    fn guarantees(&self) -> StoreGuarantees {
        (**self).guarantees()
    }
}

impl<T: Clock + ?Sized> Clock for Arc<T> {
    fn now(&self) -> Timestamp {
        (**self).now()
    }
}

/// What a backend does and does not promise.
///
/// Stated as data rather than prose so a composition root can refuse to start
/// when a deployment needs a guarantee the configured backend lacks, instead of
/// discovering it after losing data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct StoreGuarantees {
    /// Records survive an orderly process restart.
    pub durable_across_restart: bool,
    /// Records are visible to other processes sharing the backend.
    pub visible_across_processes: bool,
    /// A write either fully applies or does not apply if the process is killed
    /// mid-write. `false` means unproven, not proven false.
    pub crash_atomic: bool,
}

impl StoreGuarantees {
    /// The honest shape for an in-process store: nothing survives exit.
    #[must_use]
    pub const fn in_process() -> Self {
        Self {
            durable_across_restart: false,
            visible_across_processes: false,
            crash_atomic: false,
        }
    }
}
