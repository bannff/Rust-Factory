# Tasks: Evaluation

## Implemented

- [x] 1. Preserve Evaluation-owned V1 models, validation, canonical encodings, immutable result identity, exact logical keys, typed errors, and tenant-first ports.
- [x] 2. Add the object-safe, framework-neutral `EvaluationExecutor` and caller-polled standard-library future seam without selecting an async runtime.
- [x] 3. Add `local::DeterministicCriteriaEvaluator` as the deterministic reference executor for all three closed V1 criteria.
- [x] 4. Add the `serdes-ai-evals` feature and confined `serdes_ai_evals::SerdesAiEvalsExecutor` with V1 verdict/finding/hash parity and safe framework-error reduction.
- [x] 5. Keep core ownership of evidence/criterion digests, logical result identity, content hashes, and executor-assessment validation.
- [x] 6. Restrict `evaluation::memory` to bounded process-local immutable result storage with 1,024-per-tenant and 4,096-global maxima, no eviction, and truthful guarantees.
- [x] 7. Add closed `settings` DTOs for `local_deterministic`, `serdes_ai_evals`, and bounded `in_memory` storage; leave source loading and construction to composition.
- [x] 8. Route the unchanged three MCP tools through the injected `EvaluationService`/executor while preserving exact Policy capability mapping and safe result projections.
- [x] 9. Enforce serialized parameter-DTO size before Policy, then trusted context/exact capability authorization, then semantic validation and reader/store effects.
- [x] 10. Preserve exact V1 hashes: snapshot `400d023425c9ee77e3eb9ac40032e0871dcc3eaf6980b743f29fccdc025150eb`, definition `5c94014a3ba627135274d1cf4c9b54e2c06af1a24e396d8d6dc3c5f6ab90d401`, result `03414bc05e2c0b4aae494cc0fe12473da48fa0922f637e3836662839a5bebe72`.
- [x] 11. Add public, executor parity, memory capacity/concurrency, settings/schema, Policy-ordering, safe-egress, and object-safety contract tests.
- [x] 12. Document truthful cancellation-by-drop: callers own polling; current executors start no detached work and provide no timeout, acknowledgement, retry, cross-process cancellation, or recovery guarantee.

## Composition and acceptance still open

- [ ] 13. Implement and test the production `WorkflowEvidenceReader` bridge in a composition root without adding a `workflow` dependency to Evaluation ([#16]).
- [ ] 14. Prove runnable selected-executor composition over terminal Workflow evidence under [#16].
- [x] 15. Rerun the focused Evaluation feature matrix after the final documentation/code state, including local, memory, MCP, settings, and `serdes-ai-evals` combinations.
- [x] 16. Record fresh current-tree QA and security gate decisions for the completed implementation.
- [x] 17. Record final Rust SME decision as APPROVE for the completed implementation and documentation.
- [x] 18. Record final meta-architecture decision as APPROVE.
- [x] 19. Rerun and record the final `make check` result.

## Gate decisions

- **QA — APPROVE.** Verified `make check`, the MCP-only feature build, local/`serdes-ai-evals` parity, default-feature isolation, exact V1 vectors, and memory/settings contracts against the current tree.
- **Security — APPROVE.** Verified Policy ordering, the parameter-DTO/full-envelope ownership boundary, escaped egress, redaction, tenant/store bounds, and cancellation by drop with no detached work against the current tree.
- **Final Rust SME — APPROVE.** No Blocker or Required findings remain.
- **Final meta-architecture — APPROVE.** No Blocker or Required findings remain after the current-tree evidence re-review.

## Evidence boundary

Implemented tests cover the exact canonical vectors, cross-executor V1 parity, object-safe ports, malformed executor assessments, process-local store bounds and races, closed settings schemas, exact MCP tools/capabilities, pre-Policy DTO sizing, post-Policy semantic validation, zero domain effects on deny/failure, and bounded safe projections. `cargo test -p evaluation --all-features` and the repository-wide `make check` passed during this documentation update. This evidence does not provide a production Workflow evidence reader or runnable transport/composition proof.

Full MCP/JSON-RPC envelope bounds before buffering/deserialization, transport binding, Tokio/process lifecycle, adapter construction, and shutdown belong to a composition binary and its transport. `evaluation::memory` stores results only and SHALL NOT be treated as the missing Workflow bridge.

[#16]: https://github.com/bannff/Rust-Factory/issues/16
