# Tasks: Knowledge V1

**Status:** implemented and validated; QA/security, final Rust SME, and meta-architect **APPROVE**; focused Rust 1.88 matrix, status promotion to `implemented`, and final `make check` complete; only issue evidence, merge, and delivery pending
**Tracking:** GitHub [issue #37](https://github.com/bannff/Rust-Factory/issues/37), family `knowledge`, owning package `crates/knowledge`, Project status `In Progress`

Checked boxes record completed evidence, not intent. Tasks 1-31 are complete: implementation, Agent/Workflow migration, QA/security, final Rust SME/meta-architecture reviews, documentation reconciliation, status promotion to `implemented`, the focused Rust 1.88 matrix, and final repository-wide validation all passed with no Blocker or Required findings. Task 32 remains open for issue evidence, merge, and delivery; no merge or issue closure is claimed.

## Context and tracking

- [x] 1. Verify issue #37 exists, remains open, identifies family/package `knowledge` / `crates/knowledge`, and was initially tracked in the Software-Factory Project as `Todo`; it is now `In Progress`.
- [x] 2. Inspect repository steering, root documentation and Cargo metadata, the Canonical Brick Standard and Factory Blueprint extraction rules, issue #34 precedent, Agent's then-live `KnowledgePolicy`/`KnowledgeRequest`/`KnowledgeStore`/`StaticKnowledgeStore` path, reserved tool planning/preflight/events/output accounting, Agent MCP definition DTO, and Workflow's composition-test injection.
- [x] 3. Author the pre-implementation requirements, design, and task ledger; the original specified/blocked wording and no-package constraint are historical pre-setup evidence.

## Governance and specification re-approval

- [x] 4. Obtain Rust SME re-review of the narrow extraction eligibility rule: one demonstrated direct consumer plus explicit product mandate plus atomic one-way migration of a live provisional port; preserve the zero-consumer prohibition and Storage as the sole zero-consumer exception; cite issues #37 and #34.
- [x] 5. Resolve every Rust SME Blocker and Required finding for crate ownership, private validated models, exact limits, object safety, canonical `DocumentId` ordering, service validation, static corpus semantics, Agent/Workflow migration, closed errors, and dependency decision. Record **APPROVE** before package creation.
- [x] 6. Confirm issue #37 and its Project entry still identify the approved family, intended owner, scope, and `In Progress` state before setup begins.

## Dedicated shared setup — setup owner only

- [x] 7. Assign setup to a dedicated owner after task 5 approval. In one atomic setup change, create a minimal compile-safe `crates/knowledge` package shell containing `Cargo.toml`, `src/lib.rs`, canonical `src/{model,validation,error,port,service}.rs`, `tests/public_contract.rs`, and feature-gated `src/static.rs` exposed as `r#static` as needed. Declare `family = "knowledge"`, `role = "brick"`, `status = "specified"`, `default = []`, and Cargo feature `static`; preserve no false behavior claim and do not use `role = "core"`, `status = "scaffolded"`, or the status-only scaffold exemption.
- [x] 8. In that same atomic setup change, register `crates/knowledge` in the root workspace and README; add Cargo feature `static` to the Makefile feature matrix/isolation coverage; and update validator adapter mapping/status handling and matching validator tests without altering unrelated entries. The setup owner owns `crates/knowledge/` only for this setup change; the Knowledge implementer SHALL NOT edit shared files.
- [x] 9. Validate the now-existing package target with Cargo metadata, default compilation and dependency isolation, registry tests, formatting, and `git diff --check`. Then explicitly hand exclusive `crates/knowledge/` ownership to one implementer; status remains `specified` until final gates.

## Core and static implementation — Knowledge brick owner

- [x] 10. Replace the setup shell content with the real framework-free `model`, `validation`, `error`, `port`, and `service` contract. Implement the exact specified derives, typed constructors, getters, seven public limit constants, closed `KnowledgeError` traits, exact object-safe synchronous `KnowledgeIndex` port, and service-only construction of `KnowledgeHit`/`SearchResult`. Keep `lib.rs` exports limited to the models, constants, `KnowledgeError`, `KnowledgeIndex`, and `KnowledgeService`, plus feature-gated `r#static`; preserve the package target and shared registration from setup.
- [x] 11. Implement the exact borrowed service lifecycle `KnowledgeService<'a, I: KnowledgeIndex + ?Sized> { index: &'a I }`, `pub const fn new(index: &'a I) -> Self`, and `pub fn search(&self, request: &SearchRequest) -> Result<SearchResult, KnowledgeError>` as the sole supported consumer-facing search path. Own no adapter or process lifecycle. Trust identifier-newtype-established document-local identifier validity and constructor-established text validity; validate only request-relative tenant/namespace equality, result count, strict ascending/unique IDs, and checked aggregate bytes before all-or-nothing projection. A per-hit 16-KiB check may be a defensive assertion.
- [x] 12. Implement Cargo-feature-gated std-only `r#static::StaticKnowledgeIndex` in `src/static.rs` with exact public constructor `pub fn new(documents: Vec<KnowledgeDocument>) -> Result<Self, KnowledgeError>`, no other required public inherent operation, an immutable maximum 10,000-document/64-MiB corpus, duplicate scoped-key rejection, tenant/namespace filtering, case-sensitive substring matching, canonical `DocumentId` ordering before truncation, request-limit enforcement, a `KnowledgeIndex` implementation, and honest process-local guarantees.
- [x] 13. Add no Tantivy or other dependency in V1. Preserve the future exact Tantivy 0.26.1/default-features-disabled/RamDirectory/Rust-1.88 early-gate note as deferred design evidence only.

## Atomic Agent and Workflow migration

- [x] 14. Add `namespace: String` to Agent-owned `KnowledgePolicy`; validate it with Knowledge `NamespaceId`, include it in the capability-scope digest, and add it to all Agent MCP definition DTO/schema/conversion and definition fixtures. Keep namespace definition-owned and absent from model/caller tool arguments.
- [x] 15. Inject Knowledge's service/index seam into `LocalAgentRuntime`. Convert trusted Agent tenant/principal, policy namespace, planned query, and max-results to Knowledge types after existing capability planning and preflight.
- [x] 16. Add exact Agent-owned `KnowledgeResult { document_id, text }`; project Knowledge hits into `InvocationEvent::KnowledgeSearched`, preserve checked output accounting, and safely map the four Knowledge errors.
- [x] 17. Remove Agent's `KnowledgeRequest`, `KnowledgeStore`, and `StaticKnowledgeStore` atomically with all call-site/test migration. Leave no alias, re-export, compatibility facade, or second authoritative contract.
- [x] 18. Keep Workflow core unaware. Migrate only composition tests/fixtures that inject the real Agent runtime to use Knowledge static/service injection; make no Workflow public API, lifecycle, evidence, or port change.

## Adversarial QA

- [x] 19. Test every identifier constructor boundary and grammar case; typed `KnowledgeDocument::new` with valid identifier newtypes and empty, exact 16-KiB, and one-over text; empty/whitespace-only, exact 16-KiB, one-over, and multibyte queries; exact byte retention and no normalization; `SearchLimit` zero/one/64/65; all exact public read-only accessors; required derives through public-contract usage; and constant source-free errors.
- [x] 20. Run shared service/adapter cases for tenant/namespace isolation, principal non-disclosure, duplicate scoped corpus keys, exact 10,000-document and 64-MiB corpus bounds, case-sensitive substring behavior, request limits, deterministic repeatability, and canonical first-N `DocumentId` order.
- [x] 21. Use scripted indexes that construct only valid identifier newtypes and typed `KnowledgeDocument` values to test foreign valid tenant/namespace, duplicate IDs, nonascending IDs, too many valid documents, a constructible aggregate-over-64-KiB response assembled from multiple individually valid documents, `Unavailable`, explicit `ProtocolViolation`, and no partial projection. Separately unit-test the internal checked-add validation helper with synthetic `usize` values such as `usize::MAX` and `1` to prove overflow maps to `LimitExceeded`; do not attempt to manufacture checked-add overflow through `KnowledgeDocument` or `KnowledgeIndex` public APIs. Do not claim a safe scripted adapter can return empty/oversized text or malformed identifiers; those cases belong to the identifier/document constructor tests.
- [x] 22. Test Agent policy namespace validation, digest sensitivity, MCP schema/conversion, immutable definition-selected scope, reserved-tool planning and preflight ordering, exact event projection, output accounting, error reduction, removal of old symbols, and Workflow composition injection.
- [x] 23. Obtain adversarial `qa-tester` **APPROVE** and resolve every Blocker and Required finding.

## Security gate

- [x] 24. Review trusted tenant/principal conversion, definition-owned namespace, model/caller inability to widen scope, tenant/namespace corpus isolation, principal handling, identifier-newtype and document-constructor validity, request-relative/collection adapter-output distrust, checked ceilings, context/text/error separation, and duplicate/noncanonical fail-closed behavior.
- [x] 25. Confirm no MCP/settings/ingestion surface, filesystem/network/credential/egress path, async/runtime/lifecycle ownership, persistence/durability claim, score/provenance leak, hidden Tantivy dependency, or unsafe side effect.
- [x] 26. Obtain `security-reviewer` **APPROVE** and resolve every Blocker and Required finding.

## Final API and architecture gates

- [x] 27. Obtained final `rust-factory-sme` **APPROVE** for contract fidelity, object safety, canonical ordering, framework choice, exact one-way Agent migration, dependency isolation, honest guarantees, and future adapter seams. No Blocker or Required findings remain.
- [x] 28. Obtained final `meta-architect` **APPROVE** for issue #37 product mandate, issue #34 precedent, inward dependency direction, Agent/Workflow ownership, composition lifecycle, and consistency with the Living Factory Vision. No Blocker or Required findings remain.

## Documentation, validation, status, and delivery

- [x] 29. Reconciled Knowledge specs and applicable roadmap/API documentation with implementation evidence and promoted Cargo metadata to `status = "implemented"`. This status does not claim stable, production, durable, merged, or delivered behavior; ranking, durability, MCP, persistence, and remote behavior remain non-goals.
- [x] 30. Run the focused Rust 1.88 matrix. Evidence: Knowledge default tests passed (3 unit + 19 public-contract), Knowledge `static` tests passed (3 unit + 28 public-contract), Agent `mcp` tests passed (7 unit + 32 adversarial + 9 migration), Workflow `mcp,memory` tests passed (54), and Knowledge Clippy passed with and without `static`, all with zero failures or warnings.
- [x] 31. Run `make check` and resolve registry, adapter-isolation, formatting, Clippy, test, and feature-matrix failures. Evidence: 76 validator self-tests, 13-package registry validation, default isolation including Knowledge, formatting, complete Clippy feature matrix, workspace/default and all-feature tests, Knowledge static feature tests, and affected Agent/Workflow tests passed with zero failures. `git diff --check` passed.
- [ ] 32. Record exact command/test evidence and final decisions in issue #37. Move Project status and close/deliver only when the implementation is merged and every acceptance criterion passes.

## Deferred — separately specified and gated

- [ ] 33. Tantivy evaluation after a measured lexical-index requirement, using exact 0.26.1 with default features disabled, `RamDirectory`, and an early Rust 1.88 graph gate.
- [ ] 34. Any ranked/score-bearing retrieval contract, ingestion/mutation, settings, Knowledge MCP, async/remote/vector adapter, persistence/Storage integration, mesh, or deployable composition.
