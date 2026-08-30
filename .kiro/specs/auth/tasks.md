# Tasks: Auth V1

Tracked by GitHub [issue #31](https://github.com/bannff/Rust-Factory/issues/31), with verified membership in the GitHub project `Software-Factory`. Checkboxes record evidence, not intent. Status is **implemented / delivered**. Adversarial QA, security, final Rust SME, and final meta-architecture reviews are **APPROVE** after the direct-authority identity fix. Evidence is 41 passing Biscuit unit tests plus 18 passing public-contract tests, with focused formatting, Clippy, and dependency-tree checks passing. The full combined-tree `make check` passes, README synchronization is complete, and package metadata records `status = "implemented"`. `cargo-audit` was unavailable, so no advisory scan is claimed.

## Issue, context, and retrospective design gate

- [x] 1. Record issue #31's boundary: a token-native Auth core plus a feature-gated Biscuit adapter, with Policy migration/deletion and all consumer repointing deferred to separately gated work.
- [x] 2. Inspect the current Auth core, Biscuit adapter, public-contract tests, workspace conventions, Policy compatibility shape, and issue context. Existing source demonstrates an initial vertical slice but is not gate approval.
- [x] 3. Vet the resolved Auth+Biscuit dependency graph for the cedar regressions named by issue #31: one `serde_json` resolved at `1.0.145`; no `schemars`; no `indexmap`; no `serde_json/preserve_order`; direct Apache-2.0 license. Record that upstream `biscuit-auth` 6.0.0 declares no `rust-version`, so compatibility requires compile evidence.
- [x] 4. Record the retrospective pre-implementation `rust-factory-sme` decision as **BLOCKED**. Required corrections are: explicit authority-only trust on all rights/identity/grant policies and queries; appended-block elevation prevention; exact `verify_decision` mismatch semantics and honest non-provenance rules; request/context scope equality; exact dependency pin with default features disabled; and complete exact-boundary/runtime-limit evidence. This is not an APPROVE decision.

## Required implementation corrections — implementer

- [x] 5. Keep the exact synchronous object-safe `AuthorizationResolver` contract and prove use through `Arc<dyn AuthorizationResolver>` without leaking Biscuit types into core.
- [x] 6. Treat `TokenPresentation` as untrusted opaque bearer bytes throughout documentation and code. After cryptographic verification and before Authorizer construction/execution, decode the verified encoded authority block through `biscuit-auth` schema/conversion APIs; require exactly one direct string tenant/principal fact, preserve duplicate multiplicity, reject rule-only identity, and validate both core IDs.
- [x] 7. Keep the capability authorization policy and every tool/grant extraction query explicitly `trusting authority`. Ensure appended facts/rules cannot add identity, capability, tools, or flags while appended checks can still attenuate; identity does not depend on a derivable query.
- [x] 8. Add the exact core `verify_decision(&AuthorizationRequestV1, &AuthorizationDecisionV1) -> bool` helper. Require request/context scope equality, canonical valid grants, exact lowercase 64-hex digest shape, and recomputed digest equality for allow; keep deny non-authoritative and valid by closed shape. Document and test that a recomputed forged decision is not authenticated provenance.
- [x] 9. Make `allow_decision` reject mismatched request/correlation context instead of producing a self-consistent but wrongly scoped allow. Route any adapter construction/verification mismatch to opaque deny.
- [x] 10. Preserve closed capabilities, grants, decisions, deny reason, and four construction-error/public-code variants. Ensure public `Display` and `Debug` reveal no bearer token, key, signature, Datalog, identity, capability, timing, limit, or backend detail.
- [x] 11. Preserve exact ceilings: 65,536-byte core presentation; 8,192-byte Biscuit presentation; 16 blocks; 256 loaded facts; 64 rules; 64 checks; 512 generated facts; 100 iterations; one-second wall-clock backstop. Configure the Authorizer once so authorization and each authority-scoped extraction query use its same stored limits.
- [x] 12. Keep the resolver public-key-only and host-injected. Keep minting/private-key operations privileged, adapter-owned, absent from `AuthorizationResolver`, and absent from MCP. Make no key storage, rotation, revocation, or compromise-recovery claim.
- [x] 13. Keep `biscuit-auth = "=6.0.0"`; disable defaults in Auth and enable only the adapter-required `datalog-macro` feature. Keep the generated-schema decoding trait adapter-only through optional exact `prost = "=0.10.4"`, matching the version already resolved by `biscuit-auth`. Focused compilation demonstrated that upstream 6.0.0 builder modules do not compile with every feature disabled because `ToAnyParam` is gated behind `datalog-macro`. Rerun the graph vet after this correction.
- [x] 14. Keep Auth synchronous and runtime-free. Document composition ownership of `spawn_blocking` or the runtime-equivalent boundary; do not add Tokio, spawn work, or claim cancellation/deadline behavior.

## Adversarial QA evidence — **APPROVE**

- [x] 15. Prove the default core contract: logical-ID and tool-ID exact grammar boundaries; zero/exact/limit-plus-one tool counts; empty/exact/limit-plus-one token-presentation bytes; canonical ordering/deduplication; every capability wire name; object safety; and safe error formatting.
- [x] 16. Add fixed golden vectors for grant, allow, and deny canonical bytes and SHA-256 digests. Cover each bound field independently: tenant, principal, request, correlation, capability, every tool, and every grant boolean.
- [x] 17. Test `verify_decision` success for an untouched allow and closed deny; false for request/context mismatch, changed identity, capability, grant, stale digest, empty/short/long digest, uppercase hex, non-hex, and malformed noncanonical grant. Separately prove that a fully recomputed forged allow can pass digest consistency and therefore must still be rejected as caller-supplied provenance by consuming design.
- [x] 18. Prove signature and token failure behavior: correct root allows; wrong root, tampering, malformed base64, expiry, missing identity, distinct or identical duplicate direct identity facts, rule-only identity, invalid ID/tool grammar, unauthorized capability, and unknown grant names all collapse to the identical opaque deny.
- [x] 19. Add appended-block elevation tests for `tenant`, `principal`, `capability`, `allowed_tool`, and all four `grant` names. Each appended fact/rule SHALL fail to elevate because every read is `trusting authority`; restrictive appended checks SHALL still deny, and a valid unexpired attenuation SHALL still allow.
- [x] 20. Test each structural ceiling at exact limit and limit plus one: 8,192 token bytes, 16 blocks, 256 facts, 64 rules, and 64 checks. Isolate each cause so no other ceiling or malformed token explains the result.
- [x] 21. Exercise runtime `max_facts = 512` and `max_iterations = 100` at an accepted case and a crossing case; prove runtime-limit failures deny without panic. The max-iterations regression invokes `BiscuitAuthorizationResolver` end to end with one recursive rule and 98/100 edges, stays below token/block/fact/rule/check ceilings, proves the accepted case, and proves crossing returns only opaque `Deny`. Its private test-only resolver seam retains `max_facts = 512` and `max_iterations = 100` while using a safely nonbinding 60-second wall-clock, isolating the deterministic iteration boundary from production's independent, nondeterministic one-second backstop. Production `authorize` always uses the exact 512/100/1s limits, and every authorization/query path receives the selected limits.
- [x] 22. Add deterministic parallel-call tests showing resolver clones and `Arc<dyn AuthorizationResolver>` share no per-request state, do not cross tenant/principal/grant results, and remain `Send + Sync + 'static`.

Adversarial QA decision: **APPROVE** after the direct-authority identity fix. Evidence is 41 passing Biscuit unit tests and 18 passing public-contract tests.

## Security gate — **APPROVE**

- [x] 23. Review root-public-key injection, private-key ownership, token provenance, authority-block trust, appended-block non-elevation, attenuation checks, identity ambiguity, capability/grant derivation, unknown predicates, and mint-helper privilege boundaries.
- [x] 24. Review every ingress and engine ceiling, pre-crypto rejection order, query-limit coverage, panic behavior, concurrency isolation, and synchronous blocking ownership. Confirm no unbounded token/block/fact/rule/check/result path remains.
- [x] 25. Review opaque deny and closed errors for leakage through decisions, `Debug`, `Display`, error sources, panics, timing-dependent assertions, and vendor errors. Record explicitly that V1 has no evidence sink or durable audit claim.
- [x] 26. Review decision binding and `verify_decision`: scope mismatch must fail, digest mutation must fail, and matching unkeyed digest must not be represented as authenticity, anti-forgery, or process-boundary evidence.
- [x] 27. Security-reviewer decision: **APPROVE** with no remaining Blocker or Required findings. Focused formatting, Clippy, and dependency-tree checks pass. `cargo-audit` was unavailable, so no advisory scan is claimed.

## Final API and architecture gates — **APPROVE**

- [x] 28. Final `rust-factory-sme` review confirmed the exact object-safe API, closed models/errors, authority-scoped Datalog, dependency pin/features, ownership/concurrency model, ceilings, decision semantics, and honest guarantees. Decision: **APPROVE**.
- [x] 29. Final `meta-architect` review confirmed inward dependency direction, core/vendor isolation, the privileged mint boundary, no MCP, composition-owned runtime/key lifecycle, a clean future consumer seam, and no premature Policy migration. Decision: **APPROVE**.
- [x] 30. Confirm no gate represents Auth as a remote/process-boundary authorization service, durable token store, revocation system, evidence sink, or Policy replacement migration.

## Validation and delivery complete

- [x] 31. Run formatting and focused default-core checks:

  ```sh
  cargo fmt -p auth -- --check
  cargo test -p auth
  cargo clippy -p auth --all-targets -- -D warnings
  ```

  Result: all passed; the default public-contract suite ran 18 tests.

- [x] 32. Run focused Biscuit checks on the corrected pinned/default-disabled dependency:

  ```sh
  cargo test -p auth --features biscuit
  cargo clippy -p auth --features biscuit --all-targets -- -D warnings
  cargo tree -p auth --features biscuit --edges normal
  cargo tree -p auth --features biscuit --edges normal -i serde_json
  ```

  Result: all passed on the workspace toolchain after the direct-authority identity fix; the feature suite ran 41 Biscuit unit tests plus 18 public-contract tests. Focused formatting, Clippy, and dependency-tree checks passed. The regressions for identical duplicate direct facts, rule-only identity, and end-to-end iteration crossing passed. The normal graph contains one `serde_json` at `1.0.145`, one `prost` at `0.10.4`, and no `schemars`, `indexmap`, or `serde_json/preserve_order`. Upstream declares no `rust-version`, so this records compile evidence rather than inferring an MSRV. `cargo-audit` was unavailable, so no advisory scan is claimed.

- [x] 33. Run the full combined-tree `make check` after final Rust SME and meta-architecture approval. Result: **PASS**. This records the combined working tree without attributing unrelated concurrent changes to Auth.
- [x] 34. Complete final README synchronization. The workspace inventory records Auth as implemented while preserving the V1 non-goals and composition-owned private-key and blocking-scheduling responsibilities.
- [x] 35. Confirm package metadata records `status = "implemented"` after final Rust SME and meta-architecture approval and the full combined-tree `make check` pass.

## Explicitly deferred

- [ ] 36. Repoint Agent, Workflow, Evaluation, Memory, or Observability from Policy to Auth. Each consumer requires its own compatibility contract and deny-before-effect tests.
- [ ] 37. Delete or migrate Policy. Issue #31 does not authorize either action.
- [ ] 38. Add revocation, durable token storage, token registry, evidence/audit sink, MCP, transport/server, remote verification, process-boundary authorization, multi-key rotation, or key persistence. Each requires a separate issue and full gates.

## Official sources

- [Issue #31](https://github.com/bannff/Rust-Factory/issues/31)
- [Eclipse Biscuit cryptography](https://www.biscuitsec.org/docs/reference/cryptography/)
- [Eclipse Biscuit specifications](https://doc.biscuitsec.org/reference/specifications)
- [`biscuit-auth` 6.0.0 documentation](https://docs.rs/biscuit-auth/6.0.0/biscuit_auth/)
- [`biscuit-auth` repository](https://github.com/biscuit-auth/biscuit-rust)

External source content was rephrased for compliance with licensing restrictions.
