# Design: Storage V1

This design records the delivered V1 architecture and redb 2.6.3 private format. Persisted bytes are private adapter state, not a public interchange format; any future format migration requires its own design and tests. Final Rust SME and meta-architecture reviews are **APPROVE**, with no remaining Blocker or Required findings. The focused Storage matrix and final `make check` passed on the combined Auth+Storage tree, and package metadata records `status = "implemented"`.

The final evidence includes default, `local`, `redb`, `settings`, and all-feature Storage matrices; the all-feature suite passed 45 tests. Clippy, registry, adapter-isolation, and workspace gates passed. These results do not establish task 19's physical commit/fsync failure behavior, runnable composition, consumer migration, or any deferred capability.

## Architecture

```text
capability owner (Agent, Evaluation, or another future consumer)
  owns serialization, domain transitions, indexes, queries, audit
                         |
                         v
          framework-free storage core
       model / validation / error / port
                  ObjectStore
                   /       \
          local adapter   redb adapter
                   \       /
     settings describes selection and limits
                         ^
     composition owns source, trusted Path, construction,
     feature availability, lock/open policy, backup, shutdown
```

Core has no persistence framework and no knowledge of paths. Both adapters implement `ObjectStore` directly. `service.rs` is private and documentary unless a later requirement establishes orchestration that is neither model validation nor adapter behavior.

## Core model

The API and fixed limits are normative in [requirements.md](requirements.md). Newtypes make structurally invalid tenant IDs, namespaces, keys, values, list limits, versions, and limit configurations unrepresentable at public construction boundaries. `StorageScope { tenant_id: TenantId, namespace: Namespace }` and `ListRequest { after_key: Option<ObjectKey>, limit: ListLimit }` are validated core types composed from those validated values; the list request owns its optional cursor key. Core types do not derive Serde or Schemars. `ObjectVersion` wraps a private `NonZeroU64`; only adapters may construct it from a validated persisted/generated revision, and callers can clone it or compare equality without observing its numeric value.

Conditions and outcomes are data rather than overloaded errors. This keeps retry policy with the capability owner: Storage reports that a precondition did not hold but does not retry, merge, or interpret a conflict.

## Local reference adapter

`local::LocalObjectStore` holds one shared state behind a standard-library mutex; clones share the same state. The state contains an ordered map keyed by `(TenantId, Namespace, ObjectKey)`, a `u64` store-wide revision, exact per-tenant object/value-byte counters, and exact global counters. Holding the mutex defines each operation's linearization point and makes condition, quota, revision, and mutation one atomic critical section.

The ordered map provides the reference raw-key pagination behavior. Put computes all checked changes in locals before changing state. Delete removes the object and updates counters in the same critical section. No empty tenant/namespace counter entry remains. Poisoning and allocation failures project safely to `operation_failed`; they never trigger eviction or best-effort repair. The adapter reports `Volatile`, no cross-process sharing, operation atomicity, conditional writes, no eviction, and its configured effective limits.

## redb 2.6.3 private schema

The adapter uses three tables with byte keys/values. Table names are exact:

| Table | Key role | Value role |
|---|---|---|
| `storage_objects_v1` | full scoped object key | versioned opaque object |
| `storage_tenant_quotas_v1` | encoded tenant | exact tenant object/value-byte counters |
| `storage_metadata_v1` | one-byte closed metadata key | schema, revision, and global counters |

The schema byte is exactly `0x01`. All integers are unsigned big-endian. No bincode, Serde, host-width integer, native endian, Unicode normalization, or delimiter-based encoding appears in persisted keys or values.

### Object key encoding

```text
0x01
|| tenant_len: u16 BE || tenant ASCII bytes
|| namespace_len: u16 BE || namespace ASCII bytes
|| raw object-key bytes
```

Tenant and namespace lengths are nonzero and within 128; the remaining raw key is nonempty and at most 1,024 bytes. This encoding is injective because both text components are length-delimited. For one `StorageScope`, lexicographic encoded-key order is exactly unsigned lexicographic raw object-key order. The prefix for a list operation ends immediately after namespace bytes; the lower bound is exclusive when `request.after_key` is present. No sentinel suffix is stored.

### Object value encoding

```text
0x01 || version: u64 BE || raw object bytes
```

The version must be nonzero. The remaining bytes, including an empty remainder, are the exact object value and may not exceed 1,048,576 bytes. A malformed schema byte, zero version, or oversized remainder is corruption.

### Tenant quota records

Quota keys are:

```text
0x01 || tenant_len: u16 BE || tenant ASCII bytes
```

Quota values are exactly:

```text
0x01 || object_count: u64 BE || value_bytes: u64 BE
```

A tenant with no live objects has no quota record. Counts must be nonzero when a record exists, fit configured per-tenant ceilings, and equal the values recomputed from the object table.

### Metadata and revision records

`storage_metadata_v1` has exactly three records and no others:

| Key | Exact value |
|---:|---|
| `0x01` | `0x01` (private schema version) |
| `0x02` | `0x01 || store_revision: u64 BE` |
| `0x03` | `0x01 || global_object_count: u64 BE || global_value_bytes: u64 BE` |

A new empty database creates all three atomically with revision and counters zero. Revision zero means no version has yet been issued and is never attached to an object. A nonempty object table therefore requires a nonzero revision. The stored revision must be at least the maximum live object version; equality is not required because deletes remove objects without rolling the revision back. Revision `u64::MAX` is valid persisted state, but every later put returns `revision_exhausted` without opening a write path that can mutate state.

Revision values are never reused. A successful put uses `checked_add(1)` from the stored revision, writes that new revision into the object value and metadata, and commits both together. Delete does not advance revision. Consequently delete/recreate receives a version different from every version issued before exhaustion, preventing ABA for a stale `IfVersion` condition.

## Transactions and linearization

Every redb mutation uses one write transaction:

1. open the required object, tenant-quota, and metadata tables;
2. decode and validate the schema/revision/counters needed by the operation;
3. read the current object and evaluate the condition;
4. for a successful put, checked-increment revision and compute checked object/value-byte deltas against tenant and global ceilings; for delete, compute checked decrements;
5. stage object, quota, revision, and global-counter changes in that same transaction;
6. remove a tenant quota record if its resulting object count is zero;
7. call `set_durability(redb::Durability::Immediate)` on that write transaction;
8. commit and return success only after commit succeeds.

Condition mismatch, absence, revision exhaustion, quota breach, malformed state, checked-arithmetic failure, table/transaction failure, or any other outcome or error before commit aborts without persisted mutation or revision consumption. A physical commit/fsync failure returns `Err` and no success; V1 relies on redb's ACID contract, but the exact persisted state after that failure is not independently proven because redb exposes no practical safe fault-injection seam at that boundary. No compensating write or silent repair follows failure. Reads use one redb read transaction; that transaction is the operation's linearization point. A list derives `has_more` by reading at most `request.limit` plus one matching key in the requested `StorageScope` within the same read transaction and returns at most `request.limit` metadata entries.

V1's atomicity evidence consists of one redb write transaction for each mutation, `Durability::Immediate` selected before commit, redb's ACID transaction contract, and deterministic conflict, quota, corruption, and runtime-counter failure tests that leave no trace. Persisted-state certainty after a physical commit/fsync failure is **DEFERRED / NOT PROVEN**: redb 2.6.3 exposes no practical safe failure-injection seam at that boundary, and a bespoke abstraction is not justified solely for tests.

The adapter owns one opened redb `Database`; clones share it. The constructor does not provide cross-process operation sharing. Failure to acquire/open the exclusively owned database projects to `lock_unavailable` when it is lock contention and otherwise to a redacted closed error.

## Open-time validation

`RedbObjectStore::open(path: &Path, limits: StorageLimits)` is the only path-bearing API. `Path` and its parent directory are trusted composition input, never parsed from `settings`, tenant, namespace, key, or an external request. Storage passes that path to redb using host semantics; it does not canonicalize or root-confine it, reject relative paths, define symlink policy, secure parent-directory permissions, or prevent pathname replacement. Composition MUST perform any required root confinement, canonicalization, symlink handling, directory-permission setup, path admission, and TOCTOU mitigation before calling `RedbObjectStore::open`. The adapter stores no path in a public model or error and never includes it in `Display` or `Debug`.

Open first uses atomic `create_new` reservation to distinguish a genuinely nonexistent path from any existing file. Only a freshly reserved path whose Storage schema is absent may initialize the three tables and metadata records. An existing redb file with zero Storage tables is corrupt, remains unmodified, and is not reclassified as fresh; existing partial, foreign, or malformed state is also validated read-only and rejected without repair. Before returning a handle for a present schema, open performs these fail-closed checks:

1. require exactly the three metadata records and valid schema/version lengths;
2. scan `storage_objects_v1` once in encoded-key order;
3. decode every key and value canonically; reject invalid lengths, identifier grammar, schema bytes, zero versions, oversized values, or noncanonical encodings;
4. recompute, with checked `u64` arithmetic, every tenant's object count/value bytes, global object count/value bytes, and maximum live version;
5. stop and return `corrupt_store` as soon as the scan exceeds configured global object/byte ceilings, any tenant ceiling, or any fixed encoded component bound; thus work and retained recomputation are bounded by the configured limits;
6. scan at most `global_object_count + 1` tenant quota records (and never more than the configured global object ceiling plus one), rejecting malformed, zero, extra, missing, duplicate-equivalent, over-limit, or recomputation-mismatched records;
7. require exact equality between recomputed and metadata global counters;
8. require revision zero only for a store that has never issued a version, require a nonzero revision for nonempty objects, and require `revision >= maximum_live_version`;
9. return the handle only if all checks pass.

Open does not infer whether a high revision with few live objects came from valid deletes, so it does not require equality with the maximum live version. It does not rewrite counters, advance/reduce revision, delete malformed rows, migrate schema, or repair metadata. The scan is bounded by configured retained-state ceilings, but V1 makes no bounded recovery-time claim.

## Quota accounting

Quota calculations use checked `u64` arithmetic before any mutation. Create applies `(+1, +new_bytes)` to tenant and global counters. Replace applies no object-count change and uses separate checked subtraction/addition rather than casting through a signed or platform-sized integer. Delete applies `(-1, -old_bytes)`. Internal underflow or disagreement is `corrupt_store`, not a saturating correction. Effective settings may be tightened only when open-time state already fits; otherwise open fails closed.

Metadata overhead is not charged as value bytes. It remains bounded because every live object has fixed identifier limits, exactly one object row, and at most one quota row per nonempty tenant; the global object ceiling therefore bounds all retained rows.

## Settings and composition

With `settings`, closed Serde/Schemars DTOs describe schema version `v1`, a backend enum containing `local` and `redb`, and the four fixed-width `u64` limit fields. Unknown fields and variants are rejected. Conversion to `StorageLimits` performs semantic validation and checked conversion; schema success alone is not validation.

Settings deliberately contains no database path. A composition root reads configuration, decides whether the named feature was compiled, obtains a trusted `&Path`, chooses local or redb, establishes lock/open policy, constructs `Arc<dyn ObjectStore>`, and owns backup and orderly shutdown. The crate provides no all-adapters factory because that would defeat feature isolation and turn Storage into an adapter registry.

## Error redaction

Adapter errors map at the boundary to the closed `StorageError` codes. Internal redb errors may be retained as private diagnostic sources only if neither public formatting nor stable API exposes them. Paths, table names, encoded keys/values, tenant/namespace, revisions, counters, lock owner details, OS errors, and corruption offsets are never included in public `Display` or `Debug`. Tests SHALL exercise both formatters.

## Dependency decision

The pre-implementation Rust SME decision recorded for issue #28 is **APPROVE**. The concrete gap is durable, synchronous, serializable per-operation storage with explicit immediate durability and stable on-disk format, confined to one adapter. Exact `redb = { version = "=2.6.3", default-features = false }` fills that gap and is confined to `storage::redb`.

redb 2.6.3 declares MSRV 1.85, fitting the workspace's Rust 1.88, and has a narrow mandatory graph. Latest redb 4.2 and latest Fjall 3.1 require Rust 1.90. Compatible Fjall 2.11 is heavier and introduces LSM/background-maintenance behavior that this synchronous bounded V1 neither needs nor wants to own. No persistence dependency enters core or the local/settings modules.

## Sources

- [Issue #28](https://github.com/bannff/Rust-Factory/issues/28)
- [redb 2.6.3 documentation](https://docs.rs/redb/2.6.3/redb/)
- [redb `Durability`](https://docs.rs/redb/2.6.3/redb/enum.Durability.html)
- [redb repository and versioned manifest](https://github.com/cberner/redb/blob/v2.6.3/Cargo.toml)
- [Fjall repository](https://github.com/fjall-rs/fjall)

External source content was rephrased for compliance with licensing restrictions.
