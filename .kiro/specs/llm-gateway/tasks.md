# Tasks: LLM Gateway

**Status:** implemented and validated; delivery remains pending
**Tracking:** [Issue #34](https://github.com/bannff/Rust-Factory/issues/34), with async-port prerequisite [Issue #8](https://github.com/bannff/Rust-Factory/issues/8)

Checked items record implementation or validation evidence. Only task 37 remains open for merge/issue delivery; no checked item claims stable status, merge, PR creation, or issue closure.

## Completed context and tracking

- [x] 1. Create GitHub issue #34 with the `llm-gateway` family, intended owning crate, V1 boundary, async rationale, dependency decision, and delivery gates.
- [x] 2. Add issue #34 to the Software-Factory GitHub Project in `Todo` state.
- [x] 3. Record issue #8's prerequisite as satisfied by the concrete asynchronous `genai` 0.6.0 adapter requirement while retaining #34's approval gates.
- [x] 4. Inspect current repository steering, root README/Cargo metadata, the `model-gateway` status-only scaffold, Agent's synchronous provisional `ModelProvider`, Workflow's synchronous `AgentInvoker`/cancellation path, MCP compatibility adapters, and related tests/specifications.
- [x] 5. Specify requirements, design, exact signatures, limits, ownership, non-goals, validation, and source rationale in this directory. This records the original pre-implementation specification gate; the current status is the implemented evidence above.

## Completed conditional design approval and dedicated shared setup

- [x] 6. Obtain Rust SME conditional approval for the taxonomy boundary, exact boxed-Future APIs, object safety, core model, exact top-level `"type": "object"` schema rule, byte/item/token ceilings, error/evidence taxonomy, Agent/Workflow migration, runtime-neutral deadline factory/signal, cancellation/deadline/idempotency semantics, and non-goals. Resolve every Blocker and Required specification finding. This was the historical setup-only approval; it is not the final Rust SME `APPROVE` required by task 33.
- [x] 7. Obtain explicit setup-owner acceptance for the atomic shared-file rename plan. Do not let the brick implementer edit shared setup files.
- [x] 8. As the dedicated setup owner, perform the minimal package/manifest registration needed for dependency vetting: rename `model-gateway` to `llm-gateway`, register the workspace package and metadata, register the workspace pin `genai = { version = "=0.6.0", default-features = false }`, inherit it in the brick as optional with defaults disabled, and define exactly `genai = ["dep:genai"]`. Register validator isolation mapping only from direct dependency `genai` to module/feature `genai`; do not add direct Tokio or `futures` dependencies or mappings.
- [x] 9. Immediately after task 8 and before any adapter or core implementation, vet the exact graph on Rust 1.88:

  ```sh
  cargo +1.88.0 check -p llm-gateway --no-default-features --features genai
  cargo +1.88.0 tree -p llm-gateway --no-default-features --features genai -e features
  ```

  Inspect all resolved features/dependencies, including transitive Tokio, reqwest, and futures crates, for MSRV, unwanted defaults, and confinement. Any failure blocks implementation and overall approval; do not substitute a bespoke client.

  **Evidence:** Both commands passed on Rust 1.88. The opt-in graph resolves `genai 0.6.0`, `tokio 1.49.0`, `reqwest 0.13.4`, and `futures 0.3.34`. `genai`'s dependency declarations activate reqwest default TLS, HTTP/2, charset, and system-proxy support plus rustls/platform verification; these remain transitive and composition must inject the configured client. The default llm-gateway graph resolves only `serde_json` and no genai, network, or runtime dependency. Duplicate-major inspection found expected transitive `syn` 2/3 and macOS `core-foundation` 0.9/0.10, with no conflicting direct workspace dependency.
- [x] 10. Complete the dedicated shared setup atomically across the root workspace member, `Makefile` quality matrix, registry validator status-only allowlist and direct `genai` adapter mapping, matching validator tests, README inventory, and authoritative GitHub taxonomy/roadmap references. Preserve unrelated entries, remove stale `model-gateway` references, and register focused default, `static`, and `genai` matrix commands without enabling adapter dependencies by default.
- [x] 11. Validate the setup-only change with registry self-tests and Cargo metadata, preserve the successful task 9 evidence, and only then hand exclusive `crates/llm-gateway/` ownership to the brick implementer.

  **Evidence:** 73 validator self-tests passed; the workspace structure validator passed for 12 packages; locked offline Cargo metadata resolved the renamed package with only `static` and `genai`; default isolation passed for all 10 bricks after version-qualifying both resolved `schemars` lines; formatting and `git diff --check` passed. The implementation and final `make check` subsequently passed as recorded under tasks 12-35.

## Implemented migration

- [x] 12. Replace the status-only tree with canonical `model`, `validation`, `error`, `port`, `service`, `static`, and feature-gated `genai` modules. Keep files single-purpose and the default core free of adapter/runtime dependencies.
- [x] 13. Implement validated identifier newtypes, `Prompt`, canonical `JsonObject`, tool/request/response/evidence types, closed enums/errors, exact hard ceilings, checked aggregates, exact top-level `"type": "object"` schema acceptance, and explicit cross-field validation.
- [x] 14. Implement the exact object-safe `ProviderFuture`, `CancellationFuture`, `CancellationSignal`, `DeadlineFuture`, `DeadlineSignal`, `DeadlineFactory`, `InvocationControl`, and `LlmProvider` APIs. Add compile-time trait-object contract tests and deterministic preflight ordering tests.
- [x] 15. Implement the default-disabled std-only `static` adapter around request-independent `StaticFixture`, never a stored `GenerateResponse`. On every success call `GenerateResponse::new(actual_request, ...)` so provider/model identity and declared-tool checks are request-relative. Cover deterministic success, tool-call, normalized error, cancellation, deadline, and limit fixtures without I/O, timer, runtime, retry, or durability claims.
- [x] 16. Implement `GenaiProvider` around an injected configured client and validated non-secret provider ID using only the task 8 direct dependency/feature. Race the pinned provider operation, borrowed cancellation future, and borrowed injected deadline future with standard-library `Future` polling such as `std::future::poll_fn`; contain every vendor type/error and create no client, timer, runtime, settings, or secret resolver. Do not name Tokio, reqwest, or futures from brick source.
- [x] 17. Migrate Agent atomically from provisional `ModelProvider` contracts to `llm_gateway::LlmProvider`; construct requests only after capability-ceiling intersection and preserve Agent-owned planning, scope checks, capability ports, and normalized events/results.
- [x] 18. Add Agent-owned `InvocationModelEvidence` with exactly `provider_id`, `model_id`, `provider_request_id`, `finish_reason`, `token_usage`, and `idempotency`, using Agent-owned closed finish/usage/idempotency projections. Put `model_evidence` in `InvocationResult` and map only bounded `GenerationEvidence`; admit no raw provider data.
- [x] 19. Implement the reserved compatibility mapping for `factory.memory.recall`, `factory.memory.write`, `factory.knowledge.search`, and `factory.sandbox.execute`; reject registration collisions, malformed/unknown arguments, and denied capabilities before adapter effects.
- [x] 20. Make `LocalAgentRuntime::invoke`, `invoke_with_ceiling`, internal generation, Agent MCP `invoke_json`, and `agent_runtime_invoke` asynchronous with the specified borrowed boxed futures. Propagate one `InvocationControl` unchanged and remove superseded synchronous symbols without aliases.
- [x] 21. Migrate Workflow `AgentInvoker`, `CeilingAgentRuntime`, `PolicyAwareAgentInvoker`, `WorkflowRunner::start`/`start_with_policy`/execution, static adapters, and MCP `workflow_start` to the specified async signatures. Inject an object-safe `DeadlineFactory`; Workflow chooses the duration/key and fixed absolute `Instant`, while the factory supplies wake mechanics. Keep bounded read/list/cancel paths synchronous.
- [x] 22. Replace Workflow's polling-only cancellation flag with the runtime-neutral awaitable signal implementation, including lost-wakeup protection and drop-safe active-registration cleanup. Forward the same cancellation/deadline/key control unchanged; no library creates a timer, thread, executor, or runtime.
- [x] 23. Make `PolicyAwareAgentInvoker` canonically encode only provider/model/request-id/finish/usage/idempotency as bounded `llm_generation` evidence, emit it before `result`, and use checked per-chunk, event-count, and aggregate byte ceilings. Evidence overflow must return `LimitExceeded` and terminalize atomically without partial evidence persistence or raw provider data.
- [x] 24. Preserve exact start identity, stable downstream key, fixed absolute deadline, max-attempts one, no retry, evidence bounds, CAS terminalization, replay suppression, and tenant isolation. Keep credentials, auth refresh, endpoint/DNS policy, TLS roots/verification, proxy policy, concrete client construction, runtime-backed deadline implementation, concurrency/admission, task supervision, and shutdown outside every library. Add no `settings` or LLM Gateway `mcp` module in V1.

## Completed adversarial QA and security

- [x] 25. Run core adversarial tests for every exact limit and one over, UTF-8 byte counting, checked aggregate overflow, duplicate JSON keys, malformed/non-object schemas, missing/different/non-string/ambiguous top-level schema `type`, malformed arguments, duplicate tools, undeclared calls, inconsistent evidence identity, token mismatch/overflow, and safe error display.
- [x] 26. Run async behavior tests for pre-cancelled/pre-expired calls, cancellation versus deadline ordering, injected deadline wakeup, in-flight cancellation wakeup, dropped futures, no detached work, unchanged control propagation, and no local claim of remote abort.
- [x] 27. Run static-adapter tests proving one fixture reused with different actual requests derives request-relative provider/model identity and rejects undeclared calls each time; prove no `GenerateResponse` is stored or returned unchecked.
- [x] 28. Run Agent tests for scope-before-request ordering, reserved-name collision confinement, argument semantics, denied capability/tool behavior, exact safe `InvocationModelEvidence` projection, safe gateway error mapping, and preservation of existing output/item ceilings.
- [x] 29. Run Workflow tests for duplicate replay without re-invocation, conflicting key reuse, no retry, stable idempotency key, Workflow-selected fixed deadline passed once to the factory, cancellation/deadline acknowledgement availability, cancellation/completion/evidence-limit terminal races, canonical `llm_generation` ordering and field set, late completion suppression, no partial evidence persistence, dropped-start cleanup, and tenant isolation.
- [x] 30. Run MCP tests proving policy/context resolution before effect, direct awaiting rather than blocking, safe bounded projections, and absence of caller identity, credentials, endpoints, raw provider errors/bodies, or LLM Gateway tools.
- [x] 31. Obtain adversarial QA approval. Resolve every Blocker and Required finding.

  **Decision:** **APPROVE.** QA covered exact and one-over limits, malformed/duplicate JSON, object safety, request-relative static fixtures, provider race ordering, dropped futures, Agent scope/context/evidence behavior, Workflow replay/CAS/evidence behavior, trusted tenant isolation, and the bounded 64-subscriber broadcast cancellation implementation.
- [x] 32. Obtain security approval for capability boundaries, reserved mapping, exact object-schema validation, ceilings, pre-effect cancellation/deadline handling, egress/client ownership, idempotency honesty, canonical evidence minimization, secret/error leakage, and no unsafe side effects. Resolve every Blocker and Required finding.

  **Decision:** **APPROVE.** Security covered trusted invocation-context derivation and unchanged effect-port propagation, tenant isolation, pre-effect scope and control checks, reserved-name confinement, bounded ingress/egress/evidence, injected client and composition-owned credentials/egress/runtime/lifecycle, normalized provider errors, and the redacted `GenaiProvider` `Debug` implementation.

## Final gates and validation

- [x] 33. Obtain final `meta-architect` and Rust SME `APPROVE` decisions for contract fidelity, inward dependencies, lifecycle/composition ownership, runtime-neutral deadline injection, no hidden runtime/timer, feature isolation, and future adapter seams.

  **Final Rust SME:** **APPROVE.** **Meta-architect:** **APPROVE.** No Blocker or Required findings remain after the core service, specification, and taxonomy corrections.
- [x] 34. Run the focused feature matrix:

  ```sh
  cargo test -p llm-gateway --no-default-features
  cargo test -p llm-gateway --no-default-features --features static
  cargo test -p llm-gateway --no-default-features --features genai
  cargo +1.88.0 check -p llm-gateway --no-default-features --features genai
  cargo +1.88.0 tree -p llm-gateway --no-default-features --features genai -e features
  cargo test -p agent --no-default-features
  cargo test -p agent --no-default-features --features mcp
  cargo test -p workflow --no-default-features
  cargo test -p workflow --no-default-features --features memory
  cargo test -p workflow --no-default-features --features mcp,memory
  ```

  **Evidence:** All commands passed. Test totals were LLM Gateway default 18, `static` 20, and `genai` 30; Agent default 30 and `mcp` 35; Workflow default 23, `memory` 40, and `mcp,memory` 52. The Rust 1.88 `genai` check and feature-tree inspection also passed.

- [x] 35. Run `make check` and resolve formatting, Clippy, tests, adapter-isolation, registry, and feature-matrix failures without weakening gates.

  **Evidence:** `make check` passed after documentation reconciliation: 73 registry self-tests, 12-package structure validation, default isolation for all 10 bricks, formatting, the complete Clippy feature matrix, and workspace/feature-matrix tests.
- [x] 36. Update README/API/spec documentation and package status to match validated implementation evidence without claiming stable status.

  **Evidence:** Promoted the package to `status = "implemented"` and reconciled the LLM Gateway requirements, design, task ledger, README inventory, and status-only scaffold supersession note. The documents preserve the explicit non-guarantees and state that delivery remains pending.
- [ ] 37. Update issue #34 with the conditional setup gate, early Rust 1.88 feature-tree evidence, final approvals, and command evidence; reconcile issue #8 and close issue #34 only after the atomic migration is merged and all acceptance criteria pass.

## Acceptance evidence required for closure

- No `model-gateway` family/path/API or synchronous provider compatibility symbol remains.
- Default LLM Gateway resolves no `genai`, MCP, settings, network, or async-runtime dependency.
- `static` is deterministic and std-only, stores a request-independent fixture rather than `GenerateResponse`, and revalidates against the actual request on every generation.
- `genai` is exact-pinned, contained, Rust-1.88-vetted immediately after setup, default-disabled, and enabled only by `genai = ["dep:genai"]`; only direct dependency `genai` maps to module `genai`, and brick source names no transitive Tokio, reqwest, or futures crate.
- Agent returns the exact safe `InvocationModelEvidence` projection; Workflow canonically emits bounded `llm_generation` before `result`, and evidence overflow persists no partial collector data.
- Agent and Workflow preserve scope, tenant, replay, terminal, evidence, cancellation, absolute-deadline, idempotency, and no-retry semantics through one object-safe runtime-neutral future chain.
- Workflow owns timeout duration/key/absolute `Instant`; composition injects deadline wake mechanics. No library owns credentials, endpoints, TLS/proxy policy, client construction, timers, threads, runtime, concurrency, process lifecycle, or shutdown.
- QA, security, meta-architecture, and final Rust SME gates approve; focused commands and `make check` pass.

## Sources

- [Rust Factory issue #34](https://github.com/bannff/Rust-Factory/issues/34)
- [Rust Factory issue #8](https://github.com/bannff/Rust-Factory/issues/8)
- [rust-genai v0.6.0 repository](https://github.com/jeremychone/rust-genai/tree/v0.6.0)
- [rust-genai 0.6.0 API documentation](https://docs.rs/genai/0.6.0/genai/)
- [Rig documentation](https://rig.rs/docs/)
- [rustformers/llm repository](https://github.com/rustformers/llm)

External source content was rephrased for compliance with licensing restrictions.
