# Design: Knowledge V1

**Status:** implemented and validated; QA/security, final Rust SME, and meta-architect **APPROVE**; focused Rust 1.88 matrix, status promotion to `implemented`, and final `make check` complete; only issue evidence, merge, and delivery pending
**Tracking:** GitHub [issue #37](https://github.com/bannff/Rust-Factory/issues/37), family `knowledge`, owning package `crates/knowledge`, Project status `In Progress`

## Architecture

```text
Agent definition/configuration
  KnowledgePolicy { enabled, namespace, max_results }
       | definition-owned namespace + trusted invocation tenant/principal
       v
Agent planning/preflight                          Workflow core
  factory.knowledge.search                            unaware
       | Query + SearchLimit                              |
       v                                                  | composition tests only
KnowledgeService  -- validates adapter output <----------+
       |
       v
KnowledgeIndex (object-safe synchronous port)
       ^
       |
r#static::StaticKnowledgeIndex
  immutable, scoped, bounded, process-local corpus
```

Knowledge owns validated retrieval models, the adapter port, service invariants, safe errors, and the static reference adapter. Agent owns why retrieval is available, which configured namespace applies, trusted-context conversion, model-tool planning, pre-effect control checks, Agent events, and invocation output accounting. Workflow continues to invoke Agent and has no Knowledge contract.

This is a narrow live-port extraction, not a pre-consumer scaffold. `LocalAgentRuntime` is the demonstrated direct consumer, issue #37 is the explicit product mandate, and the migration removes the old Agent port atomically. Issue #34 established the same one-way ownership pattern for LLM Gateway. Storage remains the only family allowed a zero-consumer package.

## Core model

All fields are private. The public limit constants are the single source of truth for constructor, service, and static-adapter ceilings:

```rust
pub const MAX_IDENTIFIER_BYTES: usize = 128;
pub const MAX_QUERY_BYTES: usize = 16 * 1024;
pub const MAX_SEARCH_LIMIT: u32 = 64;
pub const MAX_DOCUMENT_TEXT_BYTES: usize = 16 * 1024;
pub const MAX_RESULT_TEXT_BYTES: usize = 64 * 1024;
pub const MAX_STATIC_DOCUMENTS: usize = 10_000;
pub const MAX_STATIC_TEXT_BYTES: usize = 64 * 1024 * 1024;

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct TenantId(String);
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct PrincipalId(String);
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct NamespaceId(String);
#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct DocumentId(String);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Query(String);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchLimit(NonZeroU32);

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchContext {
    tenant_id: TenantId,
    principal_id: PrincipalId,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchRequest {
    context: SearchContext,
    namespace: NamespaceId,
    query: Query,
    limit: SearchLimit,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnowledgeDocument {
    tenant_id: TenantId,
    namespace: NamespaceId,
    document_id: DocumentId,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct KnowledgeHit {
    document_id: DocumentId,
    text: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct SearchResult {
    hits: Vec<KnowledgeHit>,
}
```

The four identifier types intentionally share one exact grammar: lowercase ASCII `[a-z0-9][a-z0-9_-]{0,127}` and at most `MAX_IDENTIFIER_BYTES`. They remain distinct types so tenant, principal, namespace, and document values cannot be exchanged accidentally. Each exposes the same exact API:

```rust
pub fn new(value: impl Into<String>) -> Result<Self, KnowledgeError>;
pub fn as_str(&self) -> &str;
```

`Query` exposes that same exact constructor and accessor pair, preserves exact UTF-8 bytes, rejects values over `MAX_QUERY_BYTES`, and uses Unicode whitespace classification only to reject an all-whitespace value; it does not trim or rewrite accepted text. `SearchLimit` uses `NonZeroU32` internally, explicitly checks `MAX_SEARCH_LIMIT`, and exposes exactly:

```rust
pub fn new(value: u32) -> Result<Self, KnowledgeError>;
pub const fn get(self) -> u32;
```

The composed request types cannot fail after their components are validated:

```rust
impl SearchContext {
    pub const fn new(tenant_id: TenantId, principal_id: PrincipalId) -> Self;
    pub const fn tenant_id(&self) -> &TenantId;
    pub const fn principal_id(&self) -> &PrincipalId;
}

impl SearchRequest {
    pub const fn new(
        context: SearchContext,
        namespace: NamespaceId,
        query: Query,
        limit: SearchLimit,
    ) -> Self;
    pub const fn context(&self) -> &SearchContext;
    pub const fn namespace(&self) -> &NamespaceId;
    pub const fn query(&self) -> &Query;
    pub const fn limit(&self) -> SearchLimit;
}
```

`KnowledgeDocument` retains adapter scope so the service can reject a valid document returned for another tenant or namespace. Its public construction boundary accepts only validated identifier types:

```rust
impl KnowledgeDocument {
    pub fn new(
        tenant_id: TenantId,
        namespace: NamespaceId,
        document_id: DocumentId,
        text: impl Into<String>,
    ) -> Result<Self, KnowledgeError>;

    pub const fn tenant_id(&self) -> &TenantId;
    pub const fn namespace(&self) -> &NamespaceId;
    pub const fn document_id(&self) -> &DocumentId;
    pub fn text(&self) -> &str;
}
```

External adapters and static-corpus construction first validate identifiers through their newtype constructors and then use this typed document constructor. It establishes document-local text validity once: private fields make malformed identifiers, empty or oversized text, and changed scope bindings unrepresentable in safe Rust. Principal is not document identity in V1: Agent/Policy admits the principal before the call, while the static corpus is tenant-and-namespace scoped. A future principal-partitioned corpus requires a contract change rather than silently overloading document identity.

`KnowledgeHit` has no public constructor; the service alone constructs it through a private or `pub(crate)` path. It exposes exactly `pub const fn document_id(&self) -> &DocumentId` and `pub fn text(&self) -> &str`. `SearchResult` likewise has service-only/private construction and exposes exactly `pub fn hits(&self) -> &[KnowledgeHit]`.

`lib.rs` publicly re-exports these models, the seven public limit constants, `KnowledgeError`, `KnowledgeIndex`, and `KnowledgeService`. Feature `static` additionally exposes `r#static::StaticKnowledgeIndex`; validation helpers, service-only constructors, and implementation modules are not public API.

## Port and service

```rust
pub trait KnowledgeIndex: Send + Sync {
    fn search(
        &self,
        request: &SearchRequest,
    ) -> Result<Vec<KnowledgeDocument>, KnowledgeError>;
}
```

The trait is object-safe: it uses `&self`, has no generic method or associated type, and returns only owned validated adapter values. It remains synchronous because the demonstrated immutable local adapter performs bounded in-process work. A future remote adapter would require a separate process-boundary and async design.

`KnowledgeService` borrows either a concrete index or a trait object through one generic shape and owns no adapter or process lifecycle:

```rust
pub struct KnowledgeService<'a, I: KnowledgeIndex + ?Sized> {
    index: &'a I,
}

impl<'a, I: KnowledgeIndex + ?Sized> KnowledgeService<'a, I> {
    pub const fn new(index: &'a I) -> Self;
    pub fn search(&self, request: &SearchRequest) -> Result<SearchResult, KnowledgeError>;
}
```

Its `search` method is the only supported consumer-facing path to `SearchResult`. The raw trait exists so adapters can be implemented and tested; its `Vec<KnowledgeDocument>` is not a safe result projection.

For each adapter response, the service trusts identifier-newtype-established document-local identifier validity and constructor-established text validity and checks in one bounded pass:

1. count does not exceed the request limit;
2. every valid tenant and namespace equals the request scope;
3. document IDs are strictly ascending, which simultaneously detects duplicates;
4. checked aggregate text bytes do not exceed 64 KiB; and
5. only after every check succeeds, projection contains document ID and text values.

Count, aggregate-byte, and checked-arithmetic failures are `LimitExceeded`; foreign valid scope, duplicates, and noncanonical ordering are `ProtocolViolation`. The 16-KiB per-hit ceiling is guaranteed by `KnowledgeDocument::new` and may be defensively asserted, but a safe scripted adapter cannot construct empty or oversized text or malformed identifiers. Service tests exercise the aggregate-over-64-KiB path with multiple individually valid documents. Checked-add overflow itself is not constructible through the public model/index API, so a unit test directly exercises the internal aggregate validation helper with synthetic `usize` values such as `usize::MAX` and `1`. The service never partially projects or includes rejected values in an error.

## Canonical ordering decision

V1 orders all results by `DocumentId` ascending. This is deliberately not adapter-specific ranking.

A proposed `(rank_key, DocumentId)` contract was rejected. Even a private typed integer must cross the public port in `KnowledgeDocument` or another public adapter value so the service can check it. That would establish score/rank semantics without defining what the number means across substring, lexical-index, vector, or remote adapters. The service still could not verify that an adapter assigned the right semantic rank.

Canonical identifier order has a smaller and honest contract:

- every adapter can implement it;
- the service can verify strict order and uniqueness mechanically;
- immutable-corpus requests are deterministic; and
- results expose no score-like field.

Adapters must determine the complete eligible match order before applying `SearchLimit`. The service does not repair arbitrary ordering by sorting after the fact: if an adapter already truncated an arbitrary prefix, sorting that prefix cannot prove that a lower ID was omitted. Noncanonical output therefore fails closed.

## Static adapter

Cargo feature `static` exposes `r#static::StaticKnowledgeIndex` from `src/static.rs`. Its only required public inherent operation is:

```rust
impl StaticKnowledgeIndex {
    pub fn new(
        documents: Vec<KnowledgeDocument>,
    ) -> Result<Self, KnowledgeError>;
}
```

It implements `KnowledgeIndex`; no other public operation is required. The adapter owns an immutable ordered corpus whose documents were created through the typed `KnowledgeDocument::new` after identifier-newtype validation. Construction validates at most `MAX_STATIC_DOCUMENTS`, checked aggregate text at most `MAX_STATIC_TEXT_BYTES`, and uniqueness of `(TenantId, NamespaceId, DocumentId)`. A suitable internal representation is an ordered map keyed by the scoped triple; the representation remains private.

Search filters the ordered corpus by exact tenant and namespace, applies case-sensitive `str::contains` to the exact query, and takes at most `SearchLimit`. Because the corpus is keyed in scope/document order, filtering produces canonical `DocumentId` order before truncation. The adapter performs no mutation, ingestion, normalization, indexing, I/O, retry, or background work.

These guarantees are process-local only. A constructed instance remains deterministic while alive; drop or process restart discards it, and no persistence or recovery claim exists.

## Agent migration

Agent keeps the definition-owned policy:

```rust
pub struct KnowledgePolicy {
    pub enabled: bool,
    pub namespace: String,
    pub max_results: u32,
}

pub struct KnowledgeResult {
    pub document_id: String,
    pub text: String,
}
```

`namespace` is stored as `String` because the policy remains Agent-owned definition/configuration data, but Agent validation calls the Knowledge `NamespaceId` constructor and rejects invalid definitions. `max_results` is capped at 64 and must be nonzero when knowledge is enabled; disabled definitions may retain zero. The namespace is included in the canonical capability-scope digest beside enabled/max-results state and appears in the closed Agent MCP definition input, get projection, and generated schema. It is never accepted in `factory.knowledge.search` arguments, which remain exactly `{query: string}`.

At dispatch, after Agent has intersected the capability ceiling, planned all model calls, and run invocation-control preflight, Agent constructs:

```text
SearchContext(
  TenantId <- InvocationContextV1.tenant_id,
  PrincipalId <- InvocationContextV1.principal_id,
)
NamespaceId <- resolved KnowledgePolicy.namespace
Query       <- planned model query
SearchLimit <- resolved KnowledgePolicy.max_results
```

Agent calls `KnowledgeService::search`, projects each hit to Agent-owned `KnowledgeResult`, performs its existing checked output accounting over projected text, and emits `InvocationEvent::KnowledgeSearched { results }`. Context and namespace do not appear in the event.

The migration deleted `agent::KnowledgeRequest`, `agent::KnowledgeStore`, and `agent::StaticKnowledgeStore` in the same change that introduced the inward dependency and converted all Agent tests/fixtures. No aliases or transitional dual path remain. Agent's reserved tool, grant intersection, planning, preflight, events, and accounting remain Agent-owned.

Workflow core does not import Knowledge. Its real-composition test fixture replaced `StaticKnowledgeStore` with the Knowledge static adapter and service injected into `LocalAgentRuntime`; no Workflow model, port, lifecycle, or evidence change followed.

## Error mapping

`KnowledgeError` is a closed enum deriving exactly `Clone, Copy, Debug, Eq, PartialEq` and implementing `std::fmt::Display` plus `std::error::Error`. Its four variants, exact constant display strings, source-free behavior, and data-free representation are the complete public error contract.

Knowledge errors contain no data and map at the Agent boundary:

| Knowledge | Agent projection |
|---|---|
| `InvalidRequest` | `DefinitionError::InvalidDefinition` |
| `LimitExceeded` | `DefinitionError::LimitExceeded` |
| `Unavailable` | `DefinitionError::AdapterFailure` |
| `ProtocolViolation` | `DefinitionError::AdapterFailure` |

The mapping does not format or preserve a source. Query, scope, document, and adapter values cannot enter Agent errors.

## Framework decision

No external framework fills a demonstrated V1 gap. Standard-library ordered collections and case-sensitive substring search satisfy the immutable 10,000-document/64-MiB corpus contract.

If measurements later show that scan cost violates a specified requirement, the first candidate is exact `tantivy = { version = "=0.26.1", default-features = false }`, confined to a `tantivy` adapter and using `RamDirectory`. Dedicated setup must first prove Rust 1.88 compatibility and inspect the resolved graph. This note is not dependency approval and does not authorize Cargo changes.

Swiftide and Rig are too broad because they bring RAG/agent orchestration that Agent owns. LanceDB and Qdrant introduce vector, persistence or remote-service concerns, credentials, egress, and lifecycle without a V1 requirement. They are not fallback choices under this specification.

## Security and resource boundaries

- Host-derived tenant/principal; validated globally shared Agent definition chooses namespace; caller/model/tool cannot choose it; Policy/Agent admits principal; static adapter is not principal-partitioned; retrieval filters by trusted tenant + definition namespace; global definition visibility does not make corpus global or authorize cross-tenant data. Tenant-scoped `DefinitionStore`/principal corpus are deferred.
- Trusted tenant/principal come from Agent invocation context; model output supplies only the bounded query.
- Namespace comes from validated Agent definition policy and is fixed before model output.
- The static adapter filters tenant and namespace before matching text.
- Adapter output is trusted for identifier-newtype-established document-local identifier validity and constructor-established text validity and distrusted at request-relative scope and collection boundaries.
- `KnowledgeService` borrows its index and owns no adapter or process lifecycle.
- Request, result, corpus item, corpus byte, and checked aggregate ceilings are fixed.
- Public errors are constant and source-free.
- Context values never enter text, results, or errors.
- There is no ingestion, filesystem, network, credential, remote endpoint, persistence, runtime, or process-lifecycle surface.

## Delivery boundary

The pre-setup Rust SME **APPROVED** the narrow live-port extraction and replacement contract. A dedicated setup owner created and registered the compile-safe `role = "brick"`, `status = "specified"` package before handing `crates/knowledge/` to the implementer. The framework-free core and std-only static adapter are implemented; Agent migrated atomically with no alias or compatibility facade; and Workflow's real-composition fixtures migrated without a Workflow Knowledge dependency. Adversarial QA **APPROVED** the resulting contract and coverage, and security review **APPROVED** the trusted-context, scope, ceiling, redaction, and no-side-effect boundaries.

Focused Rust 1.88 validation passed: Knowledge default tests (3 unit + 19 public-contract), Knowledge `static` tests (3 unit + 28 public-contract), Agent `mcp` tests (7 unit + 32 adversarial + 9 migration), Workflow `mcp,memory` tests (54), and Knowledge Clippy with and without `static`, all with zero failures or warnings. Final repository-wide validation also passed: `make check` completed 76 validator self-tests, 13-package registry validation, default isolation including Knowledge, formatting, the complete Clippy feature matrix, workspace/default and all-feature tests, Knowledge static feature tests, and affected Agent/Workflow tests, with zero failures. `git diff --check` also passed.

Final Rust SME and meta-architect reviews **APPROVED** with no Blocker or Required findings. Cargo metadata is promoted to `status = "implemented"`; this records implemented and validated status, not stable, production, or durable status. Only issue evidence, merge, and delivery remain pending. This document does not claim merge, issue closure, or delivery.
