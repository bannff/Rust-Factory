# Memory Brick

Tenant-scoped agent memory behind a framework-agnostic port, with the concrete backend selected declaratively per project. The `memory` family owns the `memory` crate. GitHub [Issue #18](https://github.com/bannff/Rust-Factory/issues/18) tracks this work.

## 1. Capability and scope

1. The brick SHALL own typed memory records, their validation, a stable error taxonomy, and one consumed effect port for storage. It SHALL NOT own embeddings, graph traversal, similarity, causal inference, centrality, decay, or belief revision; those are `graph`-family concerns and are deferred.
2. The port SHALL be narrow enough that a plain key-value, SQL, or document backend can implement all of it. A capability that only some backends can serve SHALL be a separate port that only capable adapters implement, so a caller discovers the gap when composing rather than when calling.
3. The brick exposes a bounded MCP surface of five tools, specified in requirement 10.

## 2. Port contract

`port::MemoryStore` is object-safe — `&self` receivers, no generic methods, no associated error — because a composition binary must read configuration, select a backend, and hold the result as `Arc<dyn MemoryStore>`. A generic-only port would force selection into the type system, which declarative configuration cannot do.

Every implementation SHALL honour eight clauses, each covered by the shared conformance suite in `tests/adapter_contract.rs`:

1. **Tenant isolation.** No operation observes or modifies a record of another tenant, and a cross-tenant read reports absence rather than a distinguishable refusal.
2. **Namespace isolation.** A key is unique within a namespace, not across namespaces.
3. **Idempotent replace.** Writing an existing key replaces it and reports `Replaced`; it never duplicates.
4. **Query totality.** Every filter in `MemoryQuery` is honoured. An adapter that cannot push a filter down applies `MemoryQuery::matches` itself.
5. **Bounded results.** At most `limit` records are returned, counting matches rather than records examined.
6. **Validation at ingress.** The adapter itself rejects a record failing `validate_record` and a query failing `validate_query`, with the same error any other adapter would return. This is not redundant with the service: `MemoryStore` is public and the fields of `MemoryRecord` and `MemoryQuery` are public, so a composition root or peer brick can hold an adapter directly and pass `limit: u32::MAX`. Deferring to a backend's own limits would also let a vendor patch release change this brick's behaviour.
7. **Failure leaves no trace.** An operation returning `Err` SHALL NOT have applied a partial effect; a failed write leaves any previous record intact and the key writable.
8. **Bounded capacity.** A write exceeding `MAX_PARTITION_RECORDS` for its partition or `MAX_TENANT_NAMESPACES` for its tenant is refused with `LimitExceeded`. Replacing an existing key consumes no capacity and is always permitted.

**Read-your-writes is a precondition, not a declared guarantee.** Clauses 1 to 3 assume a completed `put` is observable by a later `get` on the same handle. An eventually consistent backend SHALL NOT implement this port by relaxing clause 3 quietly; it needs a port whose contract admits staleness. This is deliberately not a `StoreGuarantees` flag, because a flag would let such a backend pass composition while every caller written against clause 3 silently broke.

**Cost.** One operation's work SHALL be proportional to the requesting tenant's own partition, never to the whole store. A backend whose per-key lookup degrades as the store grows SHALL NOT be driven one key at a time inside a query loop.

## 3. Guarantees are data

`StoreGuarantees` states `durable_across_restart`, `visible_across_processes`, and `crash_atomic` so a composition root can refuse to start when a deployment needs a guarantee the configured backend lacks. `false` for `crash_atomic` means unproven, not proven false. An in-process adapter SHALL report all three false. Both current adapters are in-process; neither makes any durability claim.

## 4. Validation layering

Per requirement 8 of the [Canonical Brick Standard](../brick-standard/requirements.md):

- `serde` and `schemars` appear only in the feature-gated `settings` and `mcp` adapter modules, at configuration and external transport DTO boundaries.
- Identifiers (`TenantId`, `Namespace`, `RecordKey`, `RunId`) wrap a private field behind a fallible constructor, so an invalid one cannot exist.
- Aggregates (`MemoryRecord`, `MemoryQuery`) have public fields. `MemoryRecord::validated` is a checkpoint, not a type-level property: a caller can mutate a field afterwards. Clause 6 of the port is the enforcement point. This is a deliberate, recorded departure from a strict newtype-everything reading of requirement 8; the alternative buys no safety once ingress validation exists and makes the aggregate tedious to pattern match in exactly the code that most needs to.
- Every limit is the brick's own constant, never inherited from a vendor. A vendor patch release SHALL NOT be able to change this brick's public contract.

## 5. Tenant scoping and its honest limit

`service::MemoryContext` carries the tenant; no request type has a tenant field, so a caller handed a `&MemoryContext` cannot widen scope — widening is not expressible.

That is the whole of the guarantee. `MemoryContext::new` and `TenantId::new` are both public, so any code that can name a tenant string can mint a context. Construction is therefore a **privileged operation**, and keeping it privileged is a property of how a composition root distributes contexts, not something the brick enforces.

The MCP surface of requirement 10 therefore derives the tenant from host-established identity per requirement 7(e) of the brick standard and never builds a context from a request payload. `CapabilityV1` carries the five memory variants and `MemoryPolicyContextResolver::authorize` runs on every tool path, checking both the capability and `policy::GrantV1::memory_enabled`. The typed Rust API remains unauthorized by design: a composition root that constructs `MemoryContext` directly is the trusted party.

`MemoryError::TenantMismatch` projects publicly to `not_found`, and `Debug` is hand-written to print only the public code, so neither `{}` nor `{:?}` can confirm that a key exists in another tenant.

## 6. Adapter modules and their eligibility

A brick is one crate; adapters are feature-gated modules. No feature is enabled by default.

| Module | Feature | Eligibility met by |
|---|---|---|
| `local` | `local` | A concrete core-owned stateful port with truthful process-local semantics. Std-only, and also the reference behaviour for the port contract. |
| `agentic` | `agentic` | Implements one existing core port using `agentic-memory`. |
| `settings` | `settings` | A bounded declarative selection surface whose DTOs need `serde` and `schemars`. |
| `mcp` | `mcp` | A bounded operational surface (requirement 10). |

Three rules this brick establishes and that requirement 5 of the brick standard now records:

1. A **vendor module** is named for the vendor crate it contains (`agentic`), not for the capability.
2. A **`settings` module** owns the *shape* of a project's configuration. It SHALL NOT own the source (a file, an argument) nor the backend-to-constructor `match`; those belong to the composition binary, which is the only place that knows which adapters were compiled in. A factory in the brick would have to name every adapter and would defeat the feature gating. The module is named `settings`, not `config`, because `src/config.rs` is already assigned to a `role = "server"` package by requirement 10.
3. A **dependency-free adapter is still gated.** `local` adds no dependency, but gating it keeps it inside the validator's adapter vocabulary, so the "a core module names no adapter" path rule continues to apply to it. Un-gating it would silently remove that guard.

`MemoryBackend` is total and build-independent: it lists every backend the brick knows about whether or not this build compiled it, so the same project configuration file parses in every binary. Naming a backend that was not compiled in is a startup error the binary reports, not a parse error.

`MemorySettings::max_query_limit` is enforced by the composition root passing it to `MemoryService::with_result_ceiling`. Nothing in `settings` enforces it, and the module says so: a configuration type holding an unenforced limit is worse than having no field.

## 7. Dependency decision: `agentic-memory`

| Field | Value |
|---|---|
| Crate | `agentic-memory` |
| Pin | `=0.4.2`, `default-features = false` |
| Contained in | `crates/memory/src/agentic.rs` only |
| Gap | A cognitive-memory backend with typed event kinds and session/temporal indexes, selectable as an alternative to the std-only store. |
| Why `default-features = false` | Its defaults (`cli`, `format`, `ffi`, `v3`) pull `clap`, `clap_complete`, and `rustyline` — a CLI and a REPL — plus `memmap2`/`lz4_flex` and the crate's `unsafe` FFI paths. Disabled, the resolved graph is 50 crates and contains no `unsafe` from this vendor. |
| Async | None. The crate is fully synchronous, so this brick needs no runtime and declares no `tokio` dependency. |

**Impedance mismatch and how it is bridged.** `CognitiveEvent` has no tenant, namespace, key, tag, or metadata field; its identity is a sequential `u64` and a `session_id`. Therefore:

- Isolation is **structural**: one `MemoryGraph` per `(tenant, namespace)`, so a cross-tenant read cannot happen through a filter being wrong — there is no shared container to filter. This is stronger than a predicate over one graph and is why the extra bookkeeping is worth it.
- Keys are indexed beside the graph, mapping `RecordKey` to the assigned node id.
- Tags, metadata, and provenance are held in a sidecar beside the graph rather than encoded into `content`, so content stays exactly what the caller wrote and a term filter never matches text that only appears in an encoded header.
- The vendor's similarity, causal, centrality, decay, and belief-revision engines are **unused**: they require a feature vector this adapter never computes, and they are graph concerns rather than memory concerns.
- The brick declares its own `DIMENSION`, not the vendor's `DEFAULT_DIMENSION`, for the same reason it owns every other limit.

### 7.1 Design gate: blocked and overridden

The `rust-factory-sme` design gate **BLOCKED** this work with 4 blockers and 11 required corrections. The user **explicitly overrode** the block and directed implementation to proceed, on the grounds that a crate named `agentic-memory` belongs in the `memory` brick.

This is recorded because an override that leaves no trace is the failure mode the gate exists to prevent. Accepted risk: the framework choice was not SME-endorsed at design time. Mitigations actually in place:

- The vendor is confined to one module, enforced two ways — a source path rule in `scripts/validate_brick_registry.py` and a dependency-resolution check in `make isolation-check`.
- No vendor type appears in any public signature; every vendor error collapses to `AdapterFailure` at the module boundary.
- Both adapters run the same conformance suite, plus a cross-adapter agreement test, so the vendor cannot quietly define the brick's behaviour.
- The post-implementation `rust-factory-sme` gate reviewed the built code and returned **APPROVE** after five required corrections were applied.

Alternatives considered and rejected: `conch-core` (281 crates; pulls `ort`, `hf-hub`, ONNX, and a model download), `agent-memory` (not on crates.io, so it cannot be exact-pinned), `cel-memory` (27 crates, trait-only), `mindpalace` (HNSW plus fact graph, heavier than the port needs). `rig` is deliberately deferred: it spans model, knowledge base, memory, and telemetry, so adopting it would hollow out four bricks at once.

## 8. Test strategy

- `tests/public_contract.rs` — ungated, so the framework-free default build is itself tested: identifier grammars, record and query validation at every boundary value, the capacity rule, error projection including `Debug`, filter semantics, and the `settings` DTOs under their feature.
- `tests/service_contract.rs` — gated `local`: provenance stamping from the injected clock, tenant scoping, deployment result ceiling, and defence in depth against a scripted store that returns foreign, misaddressed, over-long, or failing results.
- `tests/adapter_contract.rs` — gated `local`: one generic conformance suite over all eight clauses, run against **every** adapter, plus a cross-adapter agreement test that compares records and ordering without sorting, and a tractability guard that a query stays cheap after the writes that break the vendor's fast path.

- `src/mcp.rs`'s own `#[cfg(test)] mod tests` — the MCP surface is tested in-module, matching `observability` and `evaluation`, because its fixtures are a policy resolver and a trusted-context source rather than a store, and because the `*_json` methods it exercises are private. `make check` runs it through `cargo test -p memory --features mcp` and `--features mcp,local`. The load-bearing cases: two-tenant isolation across all four data tools, all five refusal modes reaching no store, a test that drives the five `#[tool]` functions themselves so a miswired tool body cannot pass, the framed-wire-size bound, a worst-case page at every ceiling simultaneously, and capacity reached through the surface.

Adding an adapter means adding one `#[test]` that calls `run_conformance`. If it passes, the adapter is substitutable.

## 9. Named gaps

None of these is claimed as done. The list is numbered contiguously across sections 9 and 10.7 so a gap keeps one stable identifier; section 9 is what the capability lacks, section 10.7 what its MCP surface lacks.

1. **No durable adapter.** Both adapters are in-process and report no durability. Leases, recovery, cross-process cancellation, and exactly-once effects need their own specified adapter.
2. **`agent::MemoryStore` not migrated.** `agent` still carries a provisional `recall`/`write` port against a `Vec<String>` stub. Replacing it with this port is a separately gated one-way migration.
3. **Declarative selection is not proven end to end.** No binary consumes `settings` yet, so the path from a file to a constructed adapter is specified but unexercised. The first composition binary closes this.
4. **No supply-chain gate.** No `cargo-deny` or `cargo-audit` in `make check`, so the new pin has no advisory monitoring. Tracked as [#24](https://github.com/bannff/Rust-Factory/issues/24).
5. **`FixedClock` is a test double in a production module.** Recorded exception; it moves to a `role = "test-support"` package once two consumers need shared fixtures.
6. **Tenant count is unbounded.** Capacity is bounded per tenant, so one tenant cannot exhaust the host, but admitting unbounded tenants is a deployment admission-control concern this brick does not own.

## 10. MCP surface

Five tools, each with its own `policy::CapabilityV1` variant so a grant can permit reading without permitting mutation:

| Tool | Capability | Core call |
|---|---|---|
| `memory_remember` | `MemoryRemember` | `remember` |
| `memory_recall` | `MemoryRecall` | `recall` |
| `memory_search` | `MemorySearch` | `search` |
| `memory_forget` | `MemoryForget` | `forget` |
| `memory_status` | `MemoryStatus` | `guarantees` + `result_ceiling` |

The wire names are bound into the policy decision digest, so they are permanent contracts.

### 10.1 Authorization is capability **and** grant

`MemoryPolicyContextResolver::authorize` resolves host-derived `TrustedContextV1`, calls `PolicyResolver::authorize`, **re-derives the decision digest and rejects a mismatch**, and only then consults `effective_grant.memory_enabled`. Both the capability and the flag must hold.

The digest is an unkeyed hash over public canonical bytes, so what re-derivation buys is detection of a decision **mutated after the resolver produced it** — an intermediary flipping a grant flag. It is not a signature and does not authenticate the resolver, which is trusted by injection. Stating that precisely matters: a reader who thinks it is a signature will size their threat model wrong.

The flag is checked after digest verification, not before. `memory_enabled` is inside the digest's canonical bytes, so a flag flipped after signing changes the expected digest — verifying first means the flag is never acted on before it is proven authentic.

`memory_enabled` is not redundant with the capability. `workflow` projects it into its effective capability ceiling and `agent` intersects it into an agent's memory scope, so a surface that ignored it would be a way around a ceiling those bricks already enforce.

### 10.2 Ordering: transport ceilings before the gate, semantics after

Only transport ceilings run before authorization. Every semantic check — identifier grammar, record validity, query validity — runs after it.

A ceiling reveals nothing beyond the published schema. A validation *result* is an oracle: an unauthorized caller able to distinguish `invalid_id` from a refusal has learned its request reached the validator. So an unauthorized caller receives `unauthorized` whatever it sends.

### 10.3 The surface accepts a strict subset of core-valid records

`mcp-transport`'s frame limit is 64 KiB and overflow is **terminal** — the session closes with no error reaching the caller. `MAX_RECORD_BYTES` is ~98 KiB before escaping. So the MCP ceilings are deliberately tighter, and they are **consistent by construction**: a compile-time assertion requires

```text
MAX_MCP_QUERY_LIMIT * MAX_MCP_RECORD_PROJECTION_BYTES + DEFERRAL_RESERVE_BYTES
    <= MAX_MCP_SERIALIZED_RESULT_BYTES
```

An earlier revision chose the query limit and the response ceiling independently; six legitimately written records then made `memory_search` fail permanently. The assertion exists so that cannot recur silently.

The response ceiling is half of an **escaped tool-text** ceiling. A tool returns a `String` which the protocol embeds as a JSON string, so quotes and backslashes are escaped a second time and a worst-case response can roughly double. The brick measures that escaped text directly and keeps conservative composition headroom, but it does not claim the complete JSON-RPC envelope fits: the caller-controlled request ID is known only at the composition boundary.

That measured check is load-bearing for the projection itself, not a full-frame proof: halving is a good rule but not a strict implication, since a 28,672-byte response consisting entirely of backslashes escapes to 57,346 bytes. The measurement catches it and the result is `limit_exceeded`; the first server binary must separately enforce and test the complete outbound envelope.

Content and tag ceilings are measured on the **JSON serialized** form, not the raw string. `serde_json` escapes a control character to `\u00XX`, so 4 KiB of raw control bytes serializes to 24 KiB; a raw-length check bounds nothing. Multi-byte UTF-8 passes through unescaped and is bounded by its byte length.

### 10.4 Neither truncation nor blanket refusal

`search` returns `records`, `deferred_keys`, and `oversized_keys`.

Silently shortening a list is indistinguishable from there being no more data — the failure `MemoryService::search` refuses to commit. But refusing a whole page is not acceptable either: one record written through the typed API at full core limits would make a namespace permanently unsearchable at every limit, with no recovery.

So a record that does not fit the remaining budget is **named** in `deferred_keys`, and a record too large to project even alone is **named** in `oversized_keys`. The page is always explicitly partial and the caller always has a key to act on.

The returned page is **not** a prefix of the service's result order: the projection loop continues past a deferred record, so a later smaller record can still be admitted. That is harmless while every key is named, but a future `MemoryQuery` cursor (gap 13) SHALL NOT be built on an assumed prefix, and `deferred_keys` should then be re-derived from the cursor rather than kept as a second parallel model of partiality. `recall` of an unprojectable record returns `limit_exceeded`, never `null` — confusing "too big to send" with "not there" is the one outcome that would make this surface unsafe to build on.

### 10.5 Refusal is distinguishable from failure

`MemoryError::Unauthorized` projects to `unauthorized`, distinct from `adapter_failure`. A caller must be able to tell a permanent refusal from a transient fault or it retry-loops on a capability it will never hold. The variant carries no reason: denial, `memory_enabled = false`, a tampered digest, and a failure to establish identity are indistinguishable, so it cannot be used to probe which capabilities exist.

**Deliberate deviation from the sibling bricks.** There are three refusal contracts on this control plane:

| Brick | Denial projects to | Gate failure projects to |
|---|---|---|
| `agent` | `not_found` | `operation_failed` |
| `evaluation` | `not_found` | `operation_failed` |
| `observability` | `operation_failed` | `operation_failed` |
| `memory` | `unauthorized` | `unauthorized` |

Memory's reasoning: `not_found` buys no secrecy, because a tool's existence is already public through `tools/list`, and it costs an agent the ability to stop retrying. `observability`'s `operation_failed` is the worst of the three — it makes a permanent refusal look transient, which is precisely the retry loop this variant exists to prevent.

Memory also differs in the other direction: it collapses denial, `memory_enabled = false`, a tampered digest, and a failure to establish identity into one code, where `agent` distinguishes denial from gate failure. That is deliberate — a caller can act on none of those distinctions and each one it can observe is a probe — but it is a second axis of divergence, not only a different code.

Four contracts on one control plane is a defect regardless of which is right. Reconciliation is a breaking wire-behaviour change to three shipped surfaces and needs its own gate; it is tracked as [#20](https://github.com/bannff/Rust-Factory/issues/20), and `policy`'s contract matrix — which currently mandates the `not_found` behaviour memory departed from — is where a future surface author will look.

### 10.6 Projection

`namespace`, `key`, `kind`, `content`, `tags`, `recorded_at_micros`. Omitted: `tenant_id` (derived, never a caller's business), `metadata` (opaque and unbounded in shape), and `provenance.run_id` (identifies another actor's run).

`metadata` is not accepted on ingress either — it has no core meaning and admitting it multiplies the worst-case frame for no capability tags do not already provide.

### 10.7 MCP-specific gaps

7. **No principal scoping.** Authorization is per tenant and capability; `TrustedContextV1.principal_id` is not used to scope records. Every principal in a tenant sees every record in it.
8. **No mutation evidence.** `memory_remember` and `memory_forget` are consequential and emit no audit record. `observability` exists but this brick has no integration with it, so a destructive call leaves no trace beyond its effect.
9. **`run_id` is unverified caller input.** It is written into `Provenance` — the field the core calls what makes a learning loop auditable — and is deliberately not projected back, so a reader cannot detect forged run attribution and `search` cannot filter on it. An authorized caller can attribute a memory to another actor's run.
10. **Capacity is not discoverable, and is shared within a tenant.** `memory_status` reports the result ceiling and content limit but not `MAX_PARTITION_RECORDS` or `MAX_TENANT_NAMESPACES`, so a caller learns those only by being refused. There is no per-principal quota: one principal can consume a tenant's whole allowance and starve its peers. A replace always still succeeds, so nothing becomes unrepairable.
11. **`memory_forget` is irreversible and unaudited.** No tombstone, no event, no rate limit, and no observability integration. Today the blast radius is one process lifetime because both adapters are in-process. **Composing a durable store behind this surface SHALL require an audit seam first.**
12. **The digest detects mutation, not forgery.** `policy::decision_digest` is an unkeyed hash over public canonical bytes, so re-deriving it proves a decision was not altered after the resolver produced it. It does not authenticate the resolver, which is trusted by injection.
13. **No cursor.** `deferred_keys` names what did not fit but there is no resumable cursor, because `MemoryQuery` has no `after_key`. Adding one is a core change and separately gated.
