//! The deterministic in-process adapter and a system clock.
//!
//! Feature-gated behind `local` even though it depends only on the standard
//! library, so adapter placement and opt-in composition remain mechanically
//! enforced like every other adapter.
//!
//! This adapter is also the reference behaviour for the port contract: it is the
//! simplest thing that honours every clause, so a disagreement between it and
//! another adapter is evidence the other adapter is wrong.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::MemoryError;
use crate::model::{
    MemoryQuery, MemoryRecord, Namespace, RecordKey, TenantId, Timestamp, WriteOutcome,
};
use crate::port::{Clock, MemoryStore, StoreGuarantees};
use crate::validation::check_capacity;

/// Reads the host clock.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Timestamp {
        // A clock before the epoch is not representable, and treating that as
        // zero is preferable to panicking inside a write path.
        let micros = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |elapsed| {
                u64::try_from(elapsed.as_micros()).unwrap_or(u64::MAX)
            });
        Timestamp::from_micros(micros)
    }
}

/// A clock that returns a value the test chose.
///
/// Provided by the brick rather than each test so provenance assertions are
/// written the same way everywhere. It belongs in a shared test-support package
/// once one exists; until then a production module is the only place it can live.
#[derive(Clone, Copy, Debug)]
pub struct FixedClock(Timestamp);

impl FixedClock {
    #[must_use]
    pub const fn new(at: Timestamp) -> Self {
        Self(at)
    }
}

impl Clock for FixedClock {
    fn now(&self) -> Timestamp {
        self.0
    }
}

/// One namespace's records, keyed by record key.
type Partition = BTreeMap<String, MemoryRecord>;
/// One tenant's namespaces.
type Tenant = BTreeMap<String, Partition>;

/// Deterministic in-process store.
///
/// Named for what it guarantees: nothing survives process exit.
///
/// # Why the map is nested by tenant, then namespace
///
/// A flat map keyed by `(tenant, namespace, key)` forces every query to walk every
/// tenant's records and discard the ones that do not match. That makes one
/// tenant's query latency a function of every other tenant's write volume — a
/// coarse but real inference channel — and lets one bulk writer slow everyone
/// else's reads. Nesting keeps every operation proportional to the requesting
/// tenant's own namespace, and turns both capacity ceilings into length reads
/// rather than scans across the whole store.
#[derive(Default)]
pub struct InProcessStore {
    // One lock over every tenant. Correct and simple, and because clause 8 bounds
    // each partition and each operation touches exactly one, the work done under
    // the lock is bounded — so contention is not a denial-of-service seam.
    //
    // Poisoning is process-wide and permanent: a panic while the lock is held
    // leaves every tenant seeing `AdapterFailure` for the life of the process.
    // Ignoring poisoning to recover would be worse, because the invariant a
    // mid-write panic breaks is precisely the one callers depend on.
    tenants: Mutex<BTreeMap<String, Tenant>>,
}

impl InProcessStore {
    #[must_use]
    pub fn new() -> Self {
        Self {
            tenants: Mutex::new(BTreeMap::new()),
        }
    }
}

impl MemoryStore for InProcessStore {
    fn put(&self, record: MemoryRecord) -> Result<WriteOutcome, MemoryError> {
        // Contract clause 6: validate at ingress rather than trust the caller, so
        // every adapter refuses the same input for the same reason.
        crate::validation::validate_record(&record)?;
        let mut tenants = self
            .tenants
            .lock()
            .map_err(|_| MemoryError::AdapterFailure)?;
        let tenant = tenants
            .entry(record.tenant_id.as_str().to_owned())
            .or_default();

        // Contract clause 8, checked before any mutation so a refusal leaves the
        // store exactly as it was. Both counts are length reads over this tenant
        // alone; nothing here is proportional to the whole store.
        let partition = tenant.get(record.namespace.as_str());
        check_capacity(
            tenant.len(),
            partition.is_none(),
            partition.map_or(0, Partition::len),
            partition.is_none_or(|held| !held.contains_key(record.key.as_str())),
        )?;

        let namespace = record.namespace.as_str().to_owned();
        let key = record.key.as_str().to_owned();
        Ok(
            if tenant
                .entry(namespace)
                .or_default()
                .insert(key, record)
                .is_some()
            {
                WriteOutcome::Replaced
            } else {
                WriteOutcome::Created
            },
        )
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
        // A different tenant resolves to a different subtree, so isolation needs
        // no comparison of tenants anywhere.
        Ok(tenants
            .get(tenant_id.as_str())
            .and_then(|tenant| tenant.get(namespace.as_str()))
            .and_then(|partition| partition.get(key.as_str()))
            .cloned())
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
        // Ordered by key, because a `BTreeMap` iterates in key order, so results
        // are stable across runs and identical to the other adapter's.
        Ok(partition
            .values()
            .filter(|record| query.matches(record))
            .take(query.limit as usize)
            .cloned()
            .collect())
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
        let removed = partition.remove(key.as_str()).is_some();
        // Reclaim emptied levels, so deleting everything actually returns the
        // memory and namespace churn does not accumulate against clause 8.
        if partition.is_empty() {
            tenant.remove(namespace.as_str());
        }
        if tenant.is_empty() {
            tenants.remove(tenant_id.as_str());
        }
        Ok(removed)
    }

    fn guarantees(&self) -> StoreGuarantees {
        StoreGuarantees::in_process()
    }
}
