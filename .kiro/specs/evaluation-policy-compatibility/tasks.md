# Tasks: Evaluation Policy Compatibility

## Implemented

- [x] Keep `policy` confined to `evaluation::mcp`; core and all non-MCP adapters remain Policy-free.
- [x] Inject host-owned `TrustedContextSource` and `PolicyResolver` through `EvaluationPolicyContextResolver`.
- [x] Preserve exactly three tools and mappings: validate/`EvaluationValidate`, evaluate/`EvaluationEvaluate`, get/`EvaluationGet`.
- [x] Enforce serialized parameter-DTO size before trusted-context/Policy access.
- [x] Authorize before semantic validation so denial does not disclose semantic validity.
- [x] Perform reader/store effects only after successful authorization and semantic validation.
- [x] Verify source failure, deny, and tampered Allow decision evidence make zero reader/store calls.
- [x] Verify post-Policy invalid semantic input makes zero reader/store calls.
- [x] Preserve immutable create-or-match behavior, tenant non-disclosure, exact V1 hashes, read-only evidence behavior, and safe projections for allowed operations.
- [x] Keep Policy decisions out of canonical result content and preserve core/non-MCP public contracts.
- [x] Bound canonical result bytes, serialized result bytes, and JSON-escaped MCP tool text.
- [x] Document that Evaluation's 65,536-byte request ceiling measures a serialized parameter DTO, not a full MCP/JSON-RPC envelope.

## Composition and acceptance still open

- [ ] Add and validate full MCP/JSON-RPC envelope bounds before buffering/deserialization in the composition transport; Evaluation owns no private stdio/Tokio transport.
- [ ] Supply the production Workflow evidence bridge and runnable composition proof under [#16].
- [x] Rerun focused Evaluation tests across the final MCP feature combination.
- [x] Record fresh current-tree QA and security decisions for the completed compatibility boundary.
- [x] Record final Rust SME decision as APPROVE.
- [x] Record final meta-architecture decision as APPROVE.
- [x] Rerun and record the final `make check` result.

## Gate decisions

- **QA — APPROVE.** Verified `make check`, the MCP-only feature build, local/`serdes-ai-evals` parity, default-feature isolation, exact V1 vectors, and memory/settings contracts against the current tree.
- **Security — APPROVE.** Verified Policy ordering, the parameter-DTO/full-envelope ownership boundary, escaped egress, redaction, tenant/store bounds, and cancellation by drop with no detached work against the current tree.
- **Final Rust SME — APPROVE.** No Blocker or Required findings remain.
- **Final meta-architecture — APPROVE.** No Blocker or Required findings remain after the current-tree evidence re-review.

## Evidence boundary

The implementation and contract tests prove Policy confinement, exact capability selection, verified Allow-digest handling, DTO-size-before-Policy ordering, Policy-before-semantics ordering, zero domain effects on failure, safe egress, and unchanged V1 semantic hashes. Focused Evaluation tests and the repository-wide `make check` passed during this documentation update. They do not prove pre-deserialization envelope bounds, transport lifecycle, production Workflow evidence acquisition, or runnable composition.

Promotion, experiment execution, model judging, durable persistence, retries, and shared runtime ownership remain out of scope.

[#16]: https://github.com/bannff/Rust-Factory/issues/16
