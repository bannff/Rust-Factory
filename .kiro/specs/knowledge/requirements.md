# Requirements: Knowledge V1

**Status:** implemented and validated; QA/security, final Rust SME, and meta-architect **APPROVE**; focused Rust 1.88 matrix, status promotion to `implemented`, and final `make check` complete; only issue evidence, merge, and delivery pending
**Tracking:** GitHub [issue #37](https://github.com/bannff/Rust-Factory/issues/37), family `knowledge`, owning package `crates/knowledge`, Project status `In Progress`

## 1. Eligibility and ownership

1. Knowledge was eligible for extraction because one demonstrated direct consumer, `agent::LocalAgentRuntime`, consumed the live provisional Agent-owned `KnowledgeStore` port; issue #37 recorded an explicit product mandate for the `knowledge` family; and the approved specification required one atomic, one-way migration that removed the provisional Agent contract without aliases. Issue [#34](https://github.com/bannff/Rust-Factory/issues/34) is the precedent for extracting a live provisional port under that narrow rule.
2. The extraction does not create a general pre-consumer exception. A zero-consumer package remains prohibited. Storage remains the sole zero-consumer exception under issue #28. A new contract without a live provisional port still requires the ordinary demonstrated-consumer rule.
3. The `knowledge` family SHALL canonically own bounded knowledge retrieval in `crates/knowledge`. Agent SHALL depend inward on Knowledge after the atomic migration. Knowledge SHALL NOT depend on Agent, Workflow, Policy, MCP, a model provider, Storage, a vector database, an async runtime, a filesystem/network framework, or another adapter.
4. Before package creation, the Rust SME re-reviewed and **APPROVED** the replacement contract and narrow extraction exception. A dedicated setup owner then created and registered the compile-safe package shell before handing `crates/knowledge/` to the implementer.

## 2. Core shape

1. The default core SHALL be synchronous, local-first, transport-independent, and framework-free, with modules `model`, `validation`, `error`, `port`, and `service`.
2. `default = []`. The core SHALL contain no Serde/Schemars boundary, MCP surface, settings surface, ingestion API, async API, embedding/vector API, remote client, persistence adapter, mesh adapter, process lifecycle, or background work.
3. Core models SHALL have private fields and the exact constructors, derives, and read-only accessors specified below. Core validation, not deserialization, SHALL establish validity.
4. `lib.rs` SHALL publicly re-export only the core models and public limit constants specified below, `KnowledgeError`, `KnowledgeIndex`, and `KnowledgeService`. With feature `static`, it SHALL additionally expose module `r#static` containing `StaticKnowledgeIndex`; without that feature, no static-adapter symbol SHALL be public. Validation helpers, model construction used only by the service, and module implementation details SHALL remain private or `pub(crate)`.

## 3. Identifiers, context, and request

1. `TenantId`, `PrincipalId`, `NamespaceId`, and `DocumentId` SHALL each wrap a private `String`, derive exactly `Clone, Debug, Eq, Ord, PartialEq, PartialOrd`, and expose exactly `pub fn new(value: impl Into<String>) -> Result<Self, KnowledgeError>` and `pub fn as_str(&self) -> &str`. Their exact grammar is nonempty lowercase ASCII `[a-z0-9][a-z0-9_-]{0,127}` and their maximum encoded length is `MAX_IDENTIFIER_BYTES: usize = 128`. No normalization, case folding, trimming, or Unicode equivalence is performed.
2. `SearchContext` SHALL contain exactly a validated `TenantId` and `PrincipalId`, derive exactly `Clone, Debug, Eq, PartialEq`, and expose exactly `pub const fn new(tenant_id: TenantId, principal_id: PrincipalId) -> Self`, `pub const fn tenant_id(&self) -> &TenantId`, and `pub const fn principal_id(&self) -> &PrincipalId`. It SHALL contain no request ID, correlation ID, Agent ID, Policy decision, grant, role, or caller-supplied trust marker.
3. `Query` SHALL wrap a private `String`, derive exactly `Clone, Debug, Eq, PartialEq`, and expose exactly `pub fn new(value: impl Into<String>) -> Result<Self, KnowledgeError>` and `pub fn as_str(&self) -> &str`. It SHALL contain at least one non-whitespace Unicode scalar value and at most `MAX_QUERY_BYTES: usize = 16 * 1024` bytes of UTF-8. The original bytes SHALL be retained exactly; no trimming, normalization, case folding, tokenization, or query rewriting is allowed.
4. `SearchLimit` SHALL wrap a private `NonZeroU32`, derive exactly `Clone, Copy, Debug, Eq, PartialEq`, and expose exactly `pub fn new(value: u32) -> Result<Self, KnowledgeError>` and `pub const fn get(self) -> u32`. It SHALL accept exactly `1..=MAX_SEARCH_LIMIT`, where `MAX_SEARCH_LIMIT: u32 = 64`.
5. `SearchRequest` SHALL contain exactly `SearchContext`, `NamespaceId`, `Query`, and `SearchLimit`, derive exactly `Clone, Debug, Eq, PartialEq`, and expose exactly `pub const fn new(context: SearchContext, namespace: NamespaceId, query: Query, limit: SearchLimit) -> Self`, `pub const fn context(&self) -> &SearchContext`, `pub const fn namespace(&self) -> &NamespaceId`, `pub const fn query(&self) -> &Query`, and `pub const fn limit(&self) -> SearchLimit`. Its fields SHALL remain private.
6. Tenant and principal values are trusted inputs supplied by the consuming composition path, never parsed from model output or knowledge text. Context values SHALL NOT enter document text, result text, public errors, logs owned by this capability, or adapter diagnostics exposed through the public API.
7. Global-definition trust boundary: host-derived tenant/principal; validated globally shared Agent definition chooses namespace; caller/model/tool cannot choose it; Policy/Agent admits principal; static adapter is not principal-partitioned; retrieval filters by trusted tenant + definition namespace; global definition visibility does not make corpus global or authorize cross-tenant data. Tenant-scoped DefinitionStore/principal corpus are deferred.

## 4. Adapter value and public result

1. `KnowledgeDocument` is the adapter-boundary value. It SHALL retain validated `TenantId`, `NamespaceId`, `DocumentId`, and nonempty text of at most `MAX_DOCUMENT_TEXT_BYTES: usize = 16 * 1024` bytes of UTF-8; derive exactly `Clone, Debug, Eq, PartialEq`; and expose exactly `pub fn new(tenant_id: TenantId, namespace: NamespaceId, document_id: DocumentId, text: impl Into<String>) -> Result<Self, KnowledgeError>`, `pub const fn tenant_id(&self) -> &TenantId`, `pub const fn namespace(&self) -> &NamespaceId`, `pub const fn document_id(&self) -> &DocumentId`, and `pub fn text(&self) -> &str`. External adapters and static-corpus construction SHALL first construct the validated identifier newtypes and then use this typed constructor. Private fields and the constructor SHALL prevent safe Rust from representing a document with malformed identifiers, empty or oversized text, or a changed tenant/namespace/document binding. The constructor SHALL perform no text normalization.
2. `KnowledgeHit` SHALL contain exactly `document_id` and `text`, derive exactly `Clone, Debug, Eq, PartialEq`, and expose exactly `pub const fn document_id(&self) -> &DocumentId` and `pub fn text(&self) -> &str`. Its fields SHALL be private. Construction SHALL be service-only through a private or `pub(crate)` constructor; no public constructor SHALL exist. It SHALL contain no tenant, principal, namespace, score, metadata, source, provenance, rank explanation, embedding, or adapter value.
3. `SearchResult` SHALL derive exactly `Clone, Debug, Eq, PartialEq` and expose exactly `pub fn hits(&self) -> &[KnowledgeHit]`. Its fields and construction SHALL be service-only/private; no public constructor SHALL exist. It SHALL contain at most 64 hits, with unique `DocumentId` values, no more than 16 KiB per text, and at most `MAX_RESULT_TEXT_BYTES: usize = 64 * 1024` checked aggregate text bytes. Arithmetic overflow is `KnowledgeError::LimitExceeded`.
4. Search is deterministic for the same validated request and immutable corpus.

## 5. Ordering and retrieval contract

1. V1 canonical ordering is strict `DocumentId` ascending order for every adapter. Adapters SHALL return the first `request.limit()` matching documents in that order. There is no semantic-rank, score, or adapter-selected ordering in V1.
2. The object-safe synchronous port SHALL be exactly:

   ```rust
   pub trait KnowledgeIndex: Send + Sync {
       fn search(
           &self,
           request: &SearchRequest,
       ) -> Result<Vec<KnowledgeDocument>, KnowledgeError>;
   }
   ```

3. The port is an adapter implementation seam. `KnowledgeService::search` SHALL be the sole supported consumer-facing search entry and the only public operation that returns `SearchResult`. The service SHALL borrow its adapter and own no adapter or process lifecycle:

   ```rust
   pub struct KnowledgeService<'a, I: KnowledgeIndex + ?Sized> {
       index: &'a I,
   }

   impl<'a, I: KnowledgeIndex + ?Sized> KnowledgeService<'a, I> {
       pub const fn new(index: &'a I) -> Self;
       pub fn search(&self, request: &SearchRequest) -> Result<SearchResult, KnowledgeError>;
   }
   ```

4. `KnowledgeService` SHALL trust identifier-newtype-established document-local identifier validity and constructor-established text validity. Before projection, it SHALL revalidate only request-relative tenant and namespace equality, result count no greater than `SearchLimit`, strict ascending and therefore unique `DocumentId` values, and a checked aggregate text ceiling of 64 KiB. A foreign valid scope, duplicate, or noncanonical order is `ProtocolViolation`; a count, aggregate byte ceiling, or checked-arithmetic breach is `LimitExceeded`. The per-hit 16-KiB ceiling is guaranteed by `KnowledgeDocument::new` and MAY be defensively asserted, but malformed document-local states are not representable in safe Rust.
5. The service cannot verify semantic relevance or ranking. Requiring a `(rank_key, DocumentId)` order would introduce a public score-like contract without a demonstrated semantic model, so V1 rejects it. Canonical `DocumentId` order is portable and mechanically checkable. A future ranked contract requires a separately approved API and migration.
6. An adapter SHALL select and order the complete eligible match set before truncating to `SearchLimit`. The service SHALL reject noncanonical output rather than sorting an arbitrary adapter prefix, because sorting after an adapter has truncated cannot prove that lower document IDs were not omitted.

## 6. Errors

1. `KnowledgeError` SHALL be a closed enum deriving exactly `Clone, Copy, Debug, Eq, PartialEq`, with exactly `InvalidRequest`, `LimitExceeded`, `Unavailable`, and `ProtocolViolation`.
2. `KnowledgeError` SHALL implement `std::fmt::Display` and `std::error::Error`. `Display` SHALL return exactly `invalid_request`, `limit_exceeded`, `unavailable`, or `protocol_violation` for the corresponding variant. Derived `Debug` SHALL expose no more information than the variant. `std::error::Error::source()` SHALL return `None`.
3. Errors SHALL carry no query, text, identifier, tenant, principal, namespace, count, byte total, backend detail, path, host value, or source error.
4. `InvalidRequest` covers invalid caller construction or semantics. `LimitExceeded` covers fixed ceilings and checked-arithmetic failure. `Unavailable` covers an adapter that cannot serve the operation. `ProtocolViolation` covers an adapter-reported protocol failure or service-detected foreign valid scope, duplicate, or noncanonical output.

## 7. Static adapter

1. Cargo feature `static` SHALL expose Rust module `r#static` from `src/static.rs`; no feature is enabled by default. The adapter SHALL use only the standard library. Its only required public inherent operation is exactly `pub fn new(documents: Vec<KnowledgeDocument>) -> Result<Self, KnowledgeError>` on `StaticKnowledgeIndex`; `StaticKnowledgeIndex` SHALL implement `KnowledgeIndex`, and no other public operation is required.
2. `StaticKnowledgeIndex::new` SHALL receive an immutable configured corpus of at most `MAX_STATIC_DOCUMENTS: usize = 10_000` `KnowledgeDocument` values, each constructed through `KnowledgeDocument::new`, and at most `MAX_STATIC_TEXT_BYTES: usize = 64 * 1024 * 1024` checked aggregate text bytes. Construction SHALL reject duplicate scoped keys `(TenantId, NamespaceId, DocumentId)` and aggregate overflow or over-limit.
3. Retrieval SHALL be a case-sensitive substring match of the exact query bytes against document text. It SHALL filter by request tenant and namespace, iterate matches in canonical `DocumentId` ascending order, and return at most the requested limit.
4. Principal remains part of the trusted request context but V1 static corpus membership is tenant-and-namespace scoped; the adapter SHALL NOT insert principal values into text or errors. Authorization and policy admission occur in Agent/composition before the Knowledge call.
5. The adapter is process-local and immutable. It claims no persistence, restart recovery, ingestion, mutation, refresh, lease, retry, cancellation, cross-process visibility, or durability.

## 8. Dependency decision

1. V1 SHALL use no Tantivy or other retrieval dependency. Exact substring matching, ordered scoped keys, and the stated corpus bounds are satisfied by standard-library collections and string search.
2. A future measured lexical-index requirement MAY evaluate exact `tantivy = { version = "=0.26.1", default-features = false }` with `RamDirectory`. Before any implementation, dedicated setup SHALL run an early Rust 1.88 compile and resolved feature-graph/MSRV gate. Failure blocks the candidate; it SHALL NOT be bypassed by relaxing the pin or compiler requirement.
3. Swiftide and Rig are rejected for V1 because their RAG/agent orchestration surfaces overlap Agent ownership. LanceDB and Qdrant are rejected because a local immutable substring corpus demonstrates no vector, remote-service, persistence, credential, egress, or lifecycle requirement. None SHALL enter Cargo or source under this specification.

## 9. Atomic Agent and Workflow migration

1. Agent SHALL retain `KnowledgePolicy` as Agent definition/configuration data and add `namespace: String`. The namespace SHALL validate with the exact Knowledge `NamespaceId` grammar, SHALL be included in the capability-scope digest, and SHALL appear in Agent MCP definition input/output DTOs and generated schema. `max_results` SHALL be at most 64 and SHALL be in `1..=64` when knowledge is enabled; a disabled policy MAY use zero. The namespace is definition/configuration-owned and SHALL never be selected or overridden by caller input, model output, tool arguments, or a Knowledge adapter.
2. Agent SHALL retain grant intersection, the reserved `factory.knowledge.search` tool definition, strict `{query: string}` argument planning, capability preflight, event construction, and output-byte accounting.
3. On execution, Agent SHALL convert `InvocationContextV1` tenant/principal, `KnowledgePolicy.namespace`, the planned query, and `KnowledgePolicy.max_results` into Knowledge `TenantId`, `PrincipalId`, `NamespaceId`, `Query`, and `SearchLimit`. A Knowledge result SHALL project into the Agent-owned exact shape:

   ```rust
   pub struct KnowledgeResult {
       pub document_id: String,
       pub text: String,
   }
   ```

   `InvocationEvent::KnowledgeSearched` SHALL contain `Vec<KnowledgeResult>` and no Knowledge core type.
4. Knowledge errors SHALL map without detail: `InvalidRequest` to Agent `DefinitionError::InvalidDefinition`, `LimitExceeded` to Agent `DefinitionError::LimitExceeded`, and `Unavailable` or `ProtocolViolation` to Agent `DefinitionError::AdapterFailure`. No context, query, document text, or adapter detail may enter the mapped error.
5. The migration SHALL remove Agent's `KnowledgeRequest`, `KnowledgeStore`, and `StaticKnowledgeStore` atomically. No alias, re-export, compatibility facade, duplicate trait, or dual authoritative path SHALL remain.
6. Workflow core SHALL remain unaware of Knowledge. Only Workflow composition tests/fixtures that construct the real Agent runtime SHALL replace Agent's static knowledge adapter with the injected Knowledge `KnowledgeService`/`StaticKnowledgeIndex` path. Workflow lifecycle, evidence, ports, and public API SHALL NOT change.

## 10. Quality, security, and delivery gates

1. After Rust SME specification re-approval, a dedicated setup owner SHALL atomically create a minimal compile-safe `crates/knowledge` package shell and perform all shared registration. The shell SHALL include `Cargo.toml`, `src/lib.rs`, canonical `src/{model,validation,error,port,service}.rs`, `tests/public_contract.rs`, and feature-gated `src/static.rs` exposed as `r#static` as needed; declare `family = "knowledge"`, `role = "brick"`, `status = "specified"`, `default = []`, and Cargo feature `static`; and make no behavior claim. In the same setup change, that owner SHALL register the package in the root workspace and README, the `static` feature in the Makefile quality matrix and adapter-isolation coverage, and the validator mapping/status handling with matching validator tests while preserving unrelated entries.
2. The setup owner SHALL validate Cargo metadata, default compilation/dependency isolation, registry tests, formatting, and `git diff --check`, then hand exclusive `crates/knowledge/` ownership to the implementer. The package SHALL NOT use `role = "core"`, `status = "scaffolded"`, or the status-only scaffold exemption; status remains `specified` until final gates. The implementer SHALL replace the shell content with the real contract and SHALL NOT edit shared setup files.
3. Adversarial QA SHALL cover exact-limit and one-over constructors; identifier grammar; `KnowledgeDocument::new` rejection of empty/oversized text; whitespace-only and multibyte queries; no normalization; duplicate scoped corpus keys; corpus item/byte ceilings; case-sensitive matching; scope isolation; request limits; canonical ordering; scripted-index foreign valid scope, duplicate/nonascending IDs, too many valid documents, a constructible aggregate-over-64-KiB response assembled from multiple individually valid documents, `Unavailable`, `ProtocolViolation`, and no partial projection; deterministic repeated search; Agent namespace digest/schema conversion; event projection; and Workflow composition injection. Scripted safe adapters SHALL construct identifiers through their public newtype constructors and documents through the typed `KnowledgeDocument::new`; they SHALL NOT claim to return empty/oversized text or malformed identifiers. A unit test of the internal checked-add validation helper SHALL separately prove arithmetic-overflow classification using synthetic `usize` values such as `usize::MAX` and `1`; checked-add overflow SHALL NOT be scripted through `KnowledgeDocument` or `KnowledgeIndex` public APIs.
4. Security review SHALL cover trusted context conversion, definition-owned namespace, tenant/namespace isolation, principal handling, context non-disclosure, adapter-output distrust at request-relative and collection boundaries, identifier-newtype and document-constructor validity, result ceilings, error redaction, absence of ingestion/egress/persistence, and no model-selected scope.
5. Final Rust SME and meta-architecture reviews SHALL approve API fidelity, inward dependencies, exact one-way migration, lifecycle ownership, framework decision, and future adapter seams before status changes.
6. Focused Cargo tests/checks and the repository `make check` SHALL pass after implementation. Passing tests alone do not approve the architecture.
7. Implementation, Agent migration, Workflow fixture migration, adversarial QA, security review, focused Rust 1.88 validation, final Rust SME/meta-architecture approval, status promotion to `implemented`, and final repository-wide `make check` are complete. No Blocker or Required findings remain. Only issue evidence, merge, and delivery remain pending; no merge or issue closure is claimed.
8. Recorded evidence: the pre-setup Rust SME **APPROVED** the narrow extraction and replacement contract; a dedicated setup owner created and registered the package shell; the implementer completed the framework-free core and std-only immutable static adapter; Agent migrated atomically with no alias or compatibility facade; Workflow's real-composition fixture migrated without a Workflow Knowledge dependency; adversarial QA and security review **APPROVED**; and the final Rust SME and meta-architect reviews **APPROVED** with no Blocker or Required findings. The focused Rust 1.88 matrix passed Knowledge default/static tests and Clippy plus Agent `mcp` and Workflow `mcp,memory` tests. Cargo metadata was promoted to `status = "implemented"`. Final repository-wide validation also passed: `make check` completed 76 validator self-tests, 13-package registry validation, default isolation including Knowledge, formatting, the complete Clippy feature matrix, workspace/default and all-feature tests, Knowledge static feature tests, and affected Agent/Workflow tests, with zero failures. `git diff --check` also passed. This evidence establishes implemented status, not stable, production, durable, merged, or delivered status.

## 11. Non-goals

Knowledge MCP; settings; ingestion or mutation; async APIs; cancellation/deadline ownership; embeddings; vector similarity; semantic ranking; score/metadata/source/provenance output; remote services; credentials or egress; persistence or Storage integration; cache; retries; durability/recovery; mesh/CRDT replication; Workflow awareness; model/provider ownership; Agent planning; Policy decisions; process topology; or a deployable binary.

## Sources

- [Issue #37: Design Knowledge capability and migrate Agent retrieval port](https://github.com/bannff/Rust-Factory/issues/37)
- [Issue #34: Build LLM Gateway capability and migrate Agent provider port](https://github.com/bannff/Rust-Factory/issues/34)
- [Canonical Brick Standard](../brick-standard/requirements.md)
- [Factory Blueprint](../factory-blueprint/requirements.md)
