# Tasks: Storage V1

Tracked by GitHub [issue #28](https://github.com/bannff/Rust-Factory/issues/28). Checkboxes record delivery evidence, not intent. Storage V1 is delivered with final Rust SME and meta-architecture **APPROVE** decisions, no remaining Blocker or Required findings, a passing focused Storage matrix, a passing final `make check` on the combined Auth+Storage tree, and `status = "implemented"`. Task 19 and every deferred item below remain unproven or out of scope.

## Approved design evidence

- [x] 1. Record the product decision that Storage is the authoritative bounded object capability and the explicit consumer-first exception; keep Cache separate and defer Agent/Evaluation migrations.
- [x] 2. Complete the pre-implementation `rust-factory-sme` gate with **APPROVE** for the exact object-safe API, ownership boundaries, synchronous concurrency model, closed errors, fixed limits, redb private schema, and dependency choice.
- [x] 3. Record the dependency rationale: exact `redb = { version = "=2.6.3", default-features = false }` is confined to `storage::redb`; it fills the demonstrated synchronous durable-adapter gap, declares MSRV 1.85, fits workspace Rust 1.88, and has a narrow mandatory graph. Latest redb 4.2 and Fjall 3.1 require Rust 1.90; compatible Fjall 2.11 is heavier and adds LSM/background-maintenance concerns not required by V1.

## Shared setup — dedicated setup agent only

- [x] 4. Register the `storage` package in the workspace, feature matrix/isolation checks, and validator in a dedicated shared setup pass without combining those changes with brick implementation.
- [x] 5. Verify issue #28 exists before implementation, assign `crates/storage/` to one brick agent, and keep shared-file edits in the setup pass.

## Core and adapters — implementer

- [x] 6. Create the framework-free `storage` core with `model`, `validation`, `error`, and `port`; implement the exact five-method `ObjectStore` contract with validated scopes and list requests, consuming-key `put`, fixed limits, private nonzero versions, conditions/outcomes, closed redacted errors, and truthful guarantees. Keep `service.rs` private and documentary because V1 has no capability-level orchestration.
- [x] 7. Add the feature-gated std-only `local` reference adapter with shared clone state, checked revision/quota accounting, raw-key ordering, failure without trace, and truthful `Volatile` guarantees.
- [x] 8. Add the feature-gated `redb` adapter with exact redb 2.6.3, exact table/key/value/metadata formats, one-transaction mutation ordering, `Durability::Immediate`, exclusive trusted-`Path` open, bounded fail-closed integrity validation, and safe error reduction.
- [x] 9. Add feature-gated closed Serde/Schemars `settings` DTOs for backend name and fixed-width limits only. Configuration source, trusted path, feature availability, constructor dispatch, lock/open policy, backup, and shutdown remain composition-owned.

## Shared conformance and focused implementation tests

- [x] 10. Run one shared adapter conformance suite unchanged against local and redb. Cover put/delete conditions; exact empty and maximum values; malformed IDs, keys, values, list limits, and limits; scope/request construction; consuming-key `put`; object safety through `Arc<dyn ObjectStore>`; and safe `Display`/`Debug` projection.
- [x] 11. Cover per-tenant and global object/value capacity boundaries, replacement growth/shrink deltas, replacement at capacity, delete quota release, checked overflow/underflow, and no eviction.
- [x] 12. Cover tenant and namespace isolation, exclusive `after_key` ordering, exact `has_more`, maximum-size pages, quiescent deterministic pagination, and the unsupported concurrent cross-page snapshot assumption.
- [x] 13. Cover read-your-writes, delete/recreate ABA resistance, revision zero, monotonic allocation, injected revision exhaustion, failed-operation no trace, and no revision consumption on conflict/not-found/error.
- [x] 14. Cover concurrent CAS, create, replacement-growth, delete, and capacity races; assert one CAS winner and no quota overshoot.

## redb-specific integrity and durability tests

- [x] 15. Add golden vectors for exact object-key, object-value, tenant-quota, schema, revision, and global-counter encodings, including order preservation and malformed/noncanonical rejection.
- [x] 16. Prove clean reopen preserves exact values, versions, ordering, counters, and reported `ImmediateCommit` guarantees; verify every successful adapter mutation selects `redb::Durability::Immediate` before commit.
- [x] 17. Prove open rejects without repair wrong/missing/extra metadata and tables, bad schemas or lengths, zero versions, revision regression, malformed scoped keys, oversized values, quota/global disagreement, arithmetic failure, and state exceeding configured limits. The bounded catalog scan and existing-empty redb cases are included.
- [x] 18. Prove exclusive lock contention maps to redacted `lock_unavailable`, releases on orderly close, and exposes no path or OS/backend detail.
- [ ] 19. **DEFERRED / NOT PROVEN.** redb 2.6.3 exposes no practical safe commit/fsync failure-injection seam, and V1 does not justify a bespoke abstraction solely for tests. Existing atomicity evidence is the single redb write transaction, `Durability::Immediate` selected before commit, redb's ACID transaction contract, and deterministic conflict, quota, corruption, and runtime-counter failure tests that leave no trace. Physical commit/fsync failure atomicity remains unproven.

## Delivery gates

- [x] 20. Adversarial `qa-tester` decision: **APPROVE** after the open-path fix; 45 all-feature tests passed across deterministic behavior, condition/state transitions, malformed ingress, capacity, pagination, concurrency, reopen behavior, and adapter substitutability.
- [x] 21. `security-reviewer` decision: **APPROVE** after the bounded catalog scan and existing-empty redb fixes. Review covered tenant/namespace isolation, opaque versions, trusted-path ownership, quotas, corruption, locking, redaction, no eviction, and failed-effect atomicity evidence.
- [x] 22. Final `rust-factory-sme` decision: **APPROVE**. Final `meta-architect` decision: **APPROVE**. No Blocker or Required findings remain for exact API fidelity, inward dependencies, honest guarantees, the private redb schema, lifecycle ownership, the absence of MCP, or future consumer seams.
- [x] 23. The focused Storage matrix passed after final review: default, `local`, `redb`, `settings`, and all-feature tests; the all-feature suite passed 45 tests. Clippy passed for the default crate and each corresponding feature combination.

  ```sh
  cargo fmt --all -- --check
  cargo clippy -p storage --all-targets -- -D warnings
  cargo clippy -p storage --features local --all-targets -- -D warnings
  cargo clippy -p storage --features redb --all-targets -- -D warnings
  cargo clippy -p storage --features settings --all-targets -- -D warnings
  cargo clippy -p storage --all-features --all-targets -- -D warnings
  cargo test -p storage
  cargo test -p storage --features local
  cargo test -p storage --features redb
  cargo test -p storage --features settings
  cargo test -p storage --all-features
  ```

- [x] 24. Final workspace gate: `make check` **PASS** on the combined Auth+Storage tree. Registry, adapter-isolation, formatting, workspace Clippy, feature-matrix Clippy, workspace tests, and feature-matrix tests passed.
- [x] 25. Finalize the Storage specifications with current implementation, approval, validation, and `status = "implemented"` evidence. Do not claim a consumer migration, runnable composition, backup, repair, multi-process sharing, physical commit/fsync fault-injection proof, or durability beyond the implemented contract.

## Deferred — each requires a separate issue and full gate

- [ ] 26. **Agent Storage consumer:** design and migrate versioned Agent definitions through an Agent-owned serialization/state-machine adapter over `ObjectStore`; preserve Agent semantics and do not move them into Storage.
- [ ] 27. **Evaluation Storage consumer:** design and migrate immutable Evaluation results through an Evaluation-owned adapter over `ObjectStore`; preserve content identity/create-or-match semantics outside Storage.
- [ ] 28. **Storage MCP:** defer unless a bounded Policy-authorized agent operation is demonstrated. V1 raw object CRUD SHALL NOT be exposed through MCP.
- [ ] 29. **Cache capability:** specify separately as non-authoritative and evictable. It SHALL NOT reuse Storage guarantees or be implemented as a Storage mode.
- [ ] 30. Any schema migration, backup/restore, corruption repair, encryption, multi-process/network adapter, cross-object transaction, watch/lease/TTL feature, or durability tier beyond this V1 contract.

## Official sources

- [Issue #28: Storage decision](https://github.com/bannff/Rust-Factory/issues/28)
- [redb 2.6.3 API](https://docs.rs/redb/2.6.3/redb/)
- [redb 2.6.3 `Durability`](https://docs.rs/redb/2.6.3/redb/enum.Durability.html)
- [redb v2.6.3 manifest](https://github.com/cberner/redb/blob/v2.6.3/Cargo.toml)
- [Fjall repository](https://github.com/fjall-rs/fjall)

External source content was rephrased for compliance with licensing restrictions.
