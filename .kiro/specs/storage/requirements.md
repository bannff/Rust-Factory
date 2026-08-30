# Requirements: Storage V1

GitHub [issue #28](https://github.com/bannff/Rust-Factory/issues/28) defines Storage as a first-class capability and the approved exception to the Canonical Brick Standard's consumer-first package rule. This specification describes the delivered V1 implementation and its acceptance contract. Final Rust SME and meta-architecture reviews are **APPROVE**, with no remaining Blocker or Required findings. The focused Storage matrix and final `make check` passed on the combined Auth+Storage tree, and package metadata records `status = "implemented"`.

Delivery evidence covers default, `local`, `redb`, `settings`, and all-feature Storage test matrices; the final all-feature suite passed 45 tests. Clippy, registry, adapter-isolation, and workspace gates also passed. This evidence does not prove task 19's physical commit/fsync failure behavior, a runnable composition, consumer migration, or any deferred capability.

## 1. Capability boundary

1. Storage SHALL own authoritative, bounded, tenant- and namespace-scoped opaque versioned objects. A successful write is authoritative state, not a disposable acceleration copy; no adapter may evict an object to reclaim capacity.
2. Storage SHALL NOT become a generic database, ORM, repository, backend registry, or capability-specific persistence API. Callers retain serialization, canonical hashes, aggregate transitions, secondary indexes, domain queries and conflicts, audit, retries, recovery, and multi-object transactions.
3. Cache remains a separate deferred capability. Cache data may be evicted without violating correctness; Storage data may not.
4. The core SHALL be synchronous, framework-free, and transport-independent. V1 SHALL have no MCP surface: raw object CRUD would bypass the owning capability's semantics, and Policy defines no Storage capability.
5. The first planned consumers are Agent definitions and Evaluation results. Each migration SHALL have its own design, compatibility, security, and acceptance gate; this specification does not authorize either migration.

## 2. Exact public core contract

`ObjectStore` SHALL be object-safe and usable as `Arc<dyn ObjectStore>`: only `&self` receivers, no generic methods, no associated types, and the following five methods exactly:

```rust
pub trait ObjectStore: Send + Sync {
    fn get(
        &self,
        scope: &StorageScope,
        key: &ObjectKey,
    ) -> Result<Option<StoredObject>, StorageError>;

    fn put(
        &self,
        scope: &StorageScope,
        key: ObjectKey,
        value: ObjectValue,
        condition: PutCondition,
    ) -> Result<PutOutcome, StorageError>;

    fn delete(
        &self,
        scope: &StorageScope,
        key: &ObjectKey,
        condition: DeleteCondition,
    ) -> Result<DeleteOutcome, StorageError>;

    fn list(
        &self,
        scope: &StorageScope,
        request: &ListRequest,
    ) -> Result<ListPage, StorageError>;

    fn guarantees(&self) -> StoreGuarantees;
}
```

The core models SHALL be:

```rust
pub struct StorageScope { pub tenant_id: TenantId, pub namespace: Namespace }
pub struct ListRequest { pub after_key: Option<ObjectKey>, pub limit: ListLimit }
pub enum PutCondition { Any, IfAbsent, IfVersion(ObjectVersion) }
pub enum DeleteCondition { Any, IfVersion(ObjectVersion) }
pub enum PutOutcome { Created { version: ObjectVersion }, Replaced { version: ObjectVersion }, Conflict }
pub enum DeleteOutcome { Deleted, NotFound, Conflict }
pub struct StoredObject { pub version: ObjectVersion, pub value: ObjectValue }
pub struct ObjectMetadata { pub key: ObjectKey, pub version: ObjectVersion, pub size_bytes: u32 }
pub struct ListPage { pub objects: Vec<ObjectMetadata>, pub has_more: bool }
```

`Any` put creates or replaces. `IfAbsent` creates only when absent. `IfVersion(v)` replaces only when the current version equals `v`. `Any` delete returns `Deleted` or `NotFound`; `IfVersion(v)` returns `Deleted`, `NotFound`, or `Conflict`. A condition mismatch is an outcome, not an operational error. Conflict outcomes SHALL NOT disclose the current version.

Each successful put SHALL allocate a fresh, nonzero `ObjectVersion` from one store-wide monotonically increasing revision. Delete SHALL NOT reuse or decrement revisions. Delete followed by recreate therefore resists ABA. `ObjectVersion` SHALL expose only `Clone` and equality; it SHALL have no public numeric accessor, ordering, `Display`, serialization, or revealing `Debug`. This prevents callers from using versions as a cross-tenant activity side channel. Revision zero is reserved. A put that would advance `u64::MAX` SHALL return `revision_exhausted` without mutation.

## 3. Validation and fixed limits

All limits are Storage-owned and use fixed-width public/configuration types. Platform-sized conversion SHALL be checked before allocation, indexing, or range construction.

| Contract | Exact V1 value | Public representation |
|---|---:|---|
| `MAX_TENANT_ID_BYTES` | 128 | `u16` length |
| `MAX_NAMESPACE_BYTES` | 128 | `u16` length |
| `MAX_OBJECT_KEY_BYTES` | 1,024 | `u16` length |
| `MAX_OBJECT_VALUE_BYTES` | 1,048,576 | `u32` length |
| `MAX_LIST_LIMIT` | 1,000 | `u32` |
| `MAX_OBJECTS_PER_TENANT` | 100,000 | `u64` |
| `MAX_VALUE_BYTES_PER_TENANT` | 1,073,741,824 | `u64` |
| `MAX_OBJECTS_GLOBAL` | 1,000,000 | `u64` |
| `MAX_VALUE_BYTES_GLOBAL` | 8,589,934,592 | `u64` |

`TenantId` and `Namespace` SHALL contain 1–128 bytes of ASCII matching `[A-Za-z0-9][A-Za-z0-9._-]*`. `ObjectKey` SHALL contain 1–1,024 arbitrary bytes and order by unsigned bytewise lexicographic order. `ObjectValue` SHALL contain 0–1,048,576 opaque bytes. `ListLimit` SHALL be in `1..=1_000`. `StorageScope` and `ListRequest` SHALL be validated core types composed only from these validated values; `ListRequest` owns its optional `after_key`. Invalid values cannot be constructed through public APIs; constructors return the corresponding closed validation error.

Because the typed API has no transport envelope, request bytes are the sum of its byte-bearing core values: a maximum-size `put` carries at most 1,049,856 bytes (`128 + 128 + 1,024 + 1,048,576`); `get`, `delete`, or a `list` whose `request.after_key` is present carries at most 1,280 bytes. A list page contains at most 1,000 metadata entries and at most 1,024,000 aggregate key bytes; it never returns object values. Adapters SHALL NOT add a second, larger request path.

`StorageLimits` SHALL contain `max_objects_per_tenant: u64`, `max_value_bytes_per_tenant: u64`, `max_objects_global: u64`, and `max_value_bytes_global: u64`. Every field SHALL be nonzero and at most its V1 maximum above; each per-tenant field SHALL be no greater than its global counterpart. These are the effective retained-state ceilings reported by `guarantees()`.

Only object value bytes count toward byte quotas. Object count and value-byte quotas are enforced both per tenant and globally. Identifier and metadata memory/disk remain bounded by their fixed per-object limits and the global object ceiling. Empty namespaces and tenants retain no independent records. No API accepts batches, extension maps, caller-selected revisions, unbounded iterators, or unbounded result counts.

A replacement computes a checked signed byte delta: the object count is unchanged, growth must fit both byte ceilings, and shrinkage releases bytes. Create increments both object counters and adds value bytes. Delete decrements both object counters and releases the deleted value bytes. Arithmetic overflow, underflow, or a ceiling breach SHALL fail before mutation.

## 4. Behavioral guarantees

1. **Byte exactness.** `get` returns exactly the bytes accepted by the successful `put`; Storage performs no encoding, normalization, compression, or interpretation of object values.
2. **Isolation.** `StorageScope` is part of object identity through its tenant and namespace. No operation may observe, list, condition against, count against the per-tenant quota of, or mutate another tenant or namespace. Cross-scope lookup is indistinguishable from absence.
3. **Read-your-writes.** After a mutation returns success, every later operation invoked through that handle or one of its clones observes that mutation unless a later completed mutation supersedes it.
4. **Per-operation linearizability.** Each `get`, `put`, `delete`, and `list` takes effect at one point between invocation and response. Conditions and quota checks participate in the same atomic operation as their mutation. V1 provides no public transaction spanning calls or objects.
5. **Pagination.** `list` returns only the requested `StorageScope`, in strictly increasing raw-key order, strictly after `request.after_key` when present, with at most `request.limit` entries. `has_more` is true exactly when another matching key existed at that operation's linearization point. Repeated pages are deterministic only while that tenant/namespace is quiescent. There is no cross-page snapshot: concurrent mutation may cause later pages to omit or include entries relative to an earlier page, but a page itself contains no duplicates and remains ordered.
6. **Failure without trace.** `Conflict`, `NotFound`, and failures from validation, condition evaluation, quota enforcement, revision exhaustion, corruption or checked arithmetic, and any other failure before commit consume no revision and change no object, counter, or metadata record. A failed delete consumes no revision. The local adapter preserves this no-trace guarantee for every `Err`. For a physical redb commit/fsync error, the API returns `Err` and no success; V1 relies on redb's ACID contract, but the exact persisted state after that failure is not independently proven because redb exposes no practical safe fault-injection seam at that boundary. A failed open does not repair or rewrite state.
7. **No eviction.** Capacity pressure returns `limit_exceeded`; it never removes or truncates retained objects. Replacements that do not increase bytes remain possible at capacity when their condition succeeds.
8. **Concurrency.** Racing compare-and-swap puts have at most one winner for one expected version. Racing creates and quota-consuming writes cannot overrun a ceiling.

## 5. Closed safe errors

`StorageError` SHALL be a closed enum with public codes exactly: `invalid_tenant_id`, `invalid_namespace`, `invalid_object_key`, `invalid_value`, `invalid_list_limit`, `invalid_limits`, `limit_exceeded`, `revision_exhausted`, `lock_unavailable`, `corrupt_store`, and `operation_failed`.

Validation errors identify only the invalid field category. `lock_unavailable` states only that exclusive open ownership could not be acquired. `corrupt_store` states only that persisted state failed integrity validation. All backend I/O, allocation, transaction, and platform-conversion failures otherwise collapse to `operation_failed`. `Display` and `Debug` SHALL expose only the public code: no path, backend/table name, key/value bytes, tenant/namespace, revision, quota usage, OS error, or internal cause. `NotFound` and `Conflict` remain operation outcomes, not errors.

## 6. Truthful guarantees

```rust
pub enum PersistenceGuarantee { Volatile, CleanRestart, ImmediateCommit }

pub struct StoreGuarantees {
    pub persistence: PersistenceGuarantee,
    pub shared_across_processes: bool,
    pub per_operation_atomic: bool,
    pub conditional_writes: bool,
    pub eviction: bool,
    pub limits: StorageLimits,
}
```

The enum is ordered by meaning only in this specification; the Rust type SHALL NOT implement `Ord`.

- `Volatile`: successful mutations are promised only for the lifetime of the live adapter state. Drop, process exit, or restart may lose all objects.
- `CleanRestart`: after a successful mutation and orderly adapter/database close, reopening the same trusted store with compatible limits preserves it. The guarantee makes no claim for abrupt process, OS, filesystem, or device failure.
- `ImmediateCommit`: every successful mutation explicitly uses immediate durable commit and returns only after that commit succeeds, subject to the OS, filesystem, and device honoring synchronization. It does not claim backup, corruption repair, bounded recovery time, multi-process sharing, distributed behavior, or multi-object transactions.

All V1 adapters SHALL report `per_operation_atomic = true`, `conditional_writes = true`, and `eviction = false`. The local adapter reports `Volatile` and `shared_across_processes = false`. The redb adapter reports `ImmediateCommit` and `shared_across_processes = false`; exclusive file locking is not shared operation.

## 7. Features, modules, and ownership

The `storage` crate SHALL have no default features.

- Core modules: `model`, `validation`, `error`, and `port`. `service.rs` remains private documentation only unless real orchestration emerges; adapters SHALL implement the port directly rather than route through an empty service facade.
- `local`: a feature-gated, standard-library-only reference adapter.
- `redb`: a feature-gated adapter containing exact `redb = { version = "=2.6.3", default-features = false }`; no other module names redb.
- `settings`: feature-gated, closed Serde/Schemars V1 backend and `StorageLimits` DTOs. It names `local` and `redb` declaratively but contains no path, configuration source, feature detection, or constructor dispatch.

The crate SHALL contain no Tokio, async runtime, transport, MCP, server, filesystem-capability wrapper, background worker, retry loop, backup system, or shutdown policy. A composition root owns the configuration source, compiled-feature availability, trusted database `Path` and parent directory, adapter selection and construction, lock/open policy, backup, lifecycle, and shutdown. The trusted-path contract includes root confinement, canonicalization, symlink policy, directory permissions, and protection against pathname replacement; Storage does not close the composition-level time-of-check/time-of-use window.

The redb adapter atomically reserves a nonexistent path with `create_new`. Only a path reserved as fresh may initialize an absent Storage schema. Any existing redb file with zero Storage tables is `corrupt_store` and remains unmodified; existing partial, foreign, or malformed state is likewise rejected without repair.

## 8. V1 non-goals

V1 does not provide cache semantics; eviction; TTL; watches; leases; retries; audit; encryption/key management; compression; secondary indexes; scans across tenants or namespaces; domain queries; caller transactions; multi-object atomicity; snapshots across pages; caller-supplied versions; version ordering; version exposure; async APIs; network or multi-process sharing; replication; mesh/distributed semantics; backup; corruption repair; bounded recovery time; schema migration; MCP; Policy capabilities; a deployable binary; Agent migration; or Evaluation migration.

## Sources

- [Issue #28: approved Storage product and adapter decision](https://github.com/bannff/Rust-Factory/issues/28)
- [Canonical Brick Standard](../brick-standard/requirements.md)
- [redb 2.6.3 API documentation](https://docs.rs/redb/2.6.3/redb/)
- [redb repository](https://github.com/cberner/redb)
- [Fjall repository](https://github.com/fjall-rs/fjall)

External source content was rephrased for compliance with licensing restrictions.
