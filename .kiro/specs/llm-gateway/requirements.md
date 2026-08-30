# Requirements: LLM Gateway

**Status:** implemented and validated; delivery remains pending
**Tracking:** [Issue #34](https://github.com/bannff/Rust-Factory/issues/34), with async-port prerequisite [Issue #8](https://github.com/bannff/Rust-Factory/issues/8)

## Purpose

The former empty `model-gateway` scaffold has been replaced by one bounded, transport-independent `llm-gateway` brick for non-streaming LLM text generation. The brick normalizes provider requests, responses, errors, invocation control, and safe evidence. Agent retains planning, trusted invocation context, authorization scope, tenant-scoped capability execution, and safe projection; Workflow retains attempt lifecycle.

## Implementation evidence

Issue #34 implementation is present in `crates/llm-gateway`, `crates/agent`, and `crates/workflow`. The core request-relative response orchestration is isolated in `crates/llm-gateway/src/service.rs`; default-disabled `static` and `genai` adapters implement the same `LlmProvider` port. Agent now accepts explicit `InvocationContextV1`, propagates it unchanged only to Agent-owned effect ports, and uses `llm_gateway::LlmProvider` plus borrowed `InvocationControl`. Workflow provides a process-local cancellation signal with a bounded 64-subscriber broadcast, fixed attempt control, replay/terminal CAS behavior, and tenant-isolation coverage. `GenaiProvider` has a manual redacted `Debug` implementation that exposes the validated provider ID but omits the injected client and client configuration.

Adversarial QA recorded **APPROVE** after exact-limit, malformed-input, scope, context/tenant isolation, evidence, race, dropped-future, and bounded-broadcast cancellation coverage. Security review recorded **APPROVE** after reviewing trusted-context derivation, tenant propagation, reserved capability confinement, pre-effect control checks, bounded ingress/egress and evidence, injected-client ownership, error/debug redaction, and absence of library-owned credentials, endpoints, timers, runtimes, or lifecycle. Final Rust SME and meta-architecture reviews each recorded **APPROVE**, with no Blocker or Required findings remaining after the core service, specification, and taxonomy corrections.

The focused matrix passed on the Rust 1.88 repository MSRV: LLM Gateway default 18 tests, `static` 20, and `genai` 30; Agent default 30 and `mcp` 35; Workflow default 23, `memory` 40, and `mcp,memory` 52. `cargo +1.88.0 check -p llm-gateway --no-default-features --features genai`, the corresponding `cargo +1.88.0 tree ... -e features` inspection, and `make check` passed. The package is promoted to `status = "implemented"`; this evidence does not claim stable status, merge, or issue closure.

## Taxonomy and delivery ownership

1. The capability family and owning crate SHALL be renamed atomically from `model-gateway` / `crates/model-gateway` to `llm-gateway` / `crates/llm-gateway`. No compatibility crate, duplicate family, or stale registry entry SHALL remain.
2. A dedicated shared-setup owner—not the brick implementer—SHALL update the root workspace member, `Makefile` feature matrix, registry validator adapter mapping and status-only allowlist, matching validator tests, README inventory, and GitHub taxonomy/roadmap references. The setup change SHALL preserve unrelated entries.
3. The brick SHALL use `family = "llm-gateway"`, `role = "brick"`, and `status = "implemented"` after implementation validation and final approval. Issue #34 and its GitHub Projects entry are the authoritative tracking records; implementation status does not claim stable status, merge, or issue closure.
4. The rename, shared setup, LLM Gateway implementation, and Agent/Workflow migration SHALL land as one pre-1.0 compatibility migration. No old synchronous provider symbols or partial dual API SHALL remain after the migration.

## Core contract

1. The framework-free core SHALL own these public types: `ModelId`, `ProviderId`, `ToolName`, `IdempotencyKey`, `ProviderRequestId`, `Prompt`, `JsonObject`, `ToolDefinition`, `GenerationLimits`, `GenerateRequest`, `ToolCall`, `TokenUsage`, `FinishReason`, `IdempotencyDisposition`, `GenerationEvidence`, `GenerateResponse`, `LlmError`, `ProviderFuture`, `CancellationFuture`, `CancellationSignal`, `DeadlineFuture`, `DeadlineSignal`, `DeadlineFactory`, `InvocationControl`, and `LlmProvider`.
2. `ModelId`, `ProviderId`, `ToolName`, `IdempotencyKey`, and `ProviderRequestId` SHALL be nonempty validated newtypes. Provider, model, request, and idempotency identifiers SHALL be at most 256 UTF-8 bytes. Tool names SHALL be at most 128 UTF-8 bytes and use the closed ASCII grammar `[A-Za-z0-9][A-Za-z0-9_.:-]*`.
3. `Prompt` SHALL contain optional system text and required input text. Each text SHALL be at most 16 KiB and their checked aggregate SHALL be at most 32 KiB.
4. A request SHALL contain no more than 64 uniquely named tools. Each tool description SHALL be at most 4 KiB. Each tool input schema SHALL be a syntactically valid JSON object, serialize canonically to at most 16 KiB, and have exactly top-level `"type": "object"`. A missing, different, non-string, or otherwise ambiguous top-level `type` SHALL return `Unsupported`. No broader object-root inference is supported in V1. All schemas in one request SHALL have a checked aggregate of at most 64 KiB.
5. A response SHALL contain at most 64 tool calls. Every call SHALL name a tool declared in the request and carry a syntactically valid JSON object. Canonical arguments SHALL be at most 16 KiB per call and 64 KiB in checked aggregate. LLM Gateway validates object shape, declaration membership, names, and bounds; Agent/tool adapters retain domain-semantic argument validation. V1 SHALL NOT claim complete arbitrary JSON Schema evaluation.
6. Response text SHALL be at most 64 KiB. Checked addition SHALL be used for prompt, schema, argument, token, and evidence aggregates; overflow SHALL return `LimitExceeded`, never wrap or saturate into acceptance.
7. `GenerationLimits::max_output_tokens` SHALL be in `1..=1_000_000`. `TokenUsage` SHALL use `u32`; reported input and output counts SHALL each be at most 1,000,000 and their checked total SHALL be at most 2,000,000. If a provider reports a total, it SHALL equal the checked sum.
8. `FinishReason` SHALL be closed to `Stop`, `Length`, `ToolCalls`, `ContentFilter`, and `Other`; `Other` SHALL not retain raw provider text. `IdempotencyDisposition` SHALL be `Unsupported` or `Accepted`.
9. `GenerationEvidence` SHALL contain only validated provider/model identity, optional bounded provider request ID, normalized finish reason, bounded token usage, and idempotency disposition. It SHALL never contain prompts, tool arguments, raw request/response bodies, headers, endpoint data, credentials, or raw provider errors.
10. `LlmError` SHALL be a closed safe taxonomy: `InvalidRequest`, `LimitExceeded`, `Unsupported`, `Cancelled`, `DeadlineExceeded`, `Authentication`, `RateLimited`, `Unavailable`, `ProviderRejected`, and `ProtocolViolation`. Display and source behavior SHALL not expose adapter internals or secrets.
11. Constructors and explicit cross-field validation SHALL establish domain validity. JSON decoding or deserialization success alone SHALL not establish semantic validity.

## Runtime-neutral asynchronous API

1. The public APIs SHALL match the signatures in [design.md](design.md): borrowed boxed `std::future::Future`s with `Send`, no `async_trait`, no runtime type, and no `'static` requirement on request data.
2. `LlmProvider`, `CancellationSignal`, `DeadlineSignal`, and `DeadlineFactory` SHALL be object-safe and SHALL be tested through trait objects.
3. Every generation receives one attempt-stable `IdempotencyKey`, one borrowed cancellation signal supporting immediate and awaitable checks, and one borrowed deadline signal exposing its absolute `Instant`, an immediate expiry check, and a borrowed awaitable elapsed future.
4. Workflow SHALL own the selected timeout duration, derive the one fixed absolute deadline for the attempt, and obtain its `DeadlineSignal` through a composition-injected `DeadlineFactory`. The factory supplies wake mechanics only: no library SHALL create timers, threads, executors, or runtimes.
5. Providers SHALL reject already-cancelled or expired calls before network effect and SHALL race in-flight work against cancellation and deadline. Cancellation wins when observed first; deadline wins when expiry is observed first. The absolute deadline SHALL NOT be reset between steps.
6. Dropping a provider future cancels only local polling/waiting. V1 makes no remote-abort, acknowledgement, exactly-once, lease, recovery, or cross-process cancellation claim.
7. LLM Gateway SHALL perform no retries. `IdempotencyDisposition` SHALL remain `Unsupported` unless the concrete provider confirms native acceptance of the supplied key; key presence alone is not evidence of acceptance.
8. No core or adapter SHALL create a runtime, call `block_on`, spawn detached work, create a timer/thread, sleep to enforce a deadline, or select process topology. The `genai` adapter SHALL race the provider, cancellation, and deadline futures with standard-library `Future` polling (for example `std::future::poll_fn`) and SHALL have no direct Tokio or `futures` dependency.

## Agent ownership and migration

1. Agent SHALL depend inward on `llm-gateway` and SHALL remove its provisional `ModelProvider`, `ModelRequest`, `ModelResponse`, `ToolCall`, `CapabilityRequest`, and `StaticModelProvider` provider contract where superseded.
2. Agent SHALL continue to own `AgentId`, definitions, model policy/reference resolution, effective capability-ceiling intersection, scope digest, `ToolRegistry`, Memory/Knowledge/Sandbox ports, planning, normalized Agent events/results, and every authorization check around capability execution.
3. Agent SHALL construct `GenerateRequest` only after resolving the definition and intersecting the effective capability ceiling. No Agent identity, trusted context, Policy decision/evidence, resolved scope, Memory/Knowledge/Sandbox request, credential, endpoint, or vendor type SHALL cross into LLM Gateway.
4. Agent SHALL map returned ordinary tool calls to Agent-owned tools only after checking the normalized name against the resolved scope. Model output SHALL never expand scope.
5. Compatibility with existing `CapabilityRequest` behavior SHALL be represented by Agent-owned reserved tool definitions and mapping: `factory.memory.recall` → `MemoryRecall { query }`, `factory.memory.write` → `MemoryWrite { value }`, `factory.knowledge.search` → `KnowledgeSearch { query }`, and `factory.sandbox.execute` → `SandboxExecute { action, arguments }`. Reserved names SHALL not be registrable as ordinary tools. Agent SHALL own their schemas, argument decoding, semantic validation, policy checks, and dispatch.
6. Agent SHALL define an Agent-owned `InvocationModelEvidence` safe projection with exactly these fields: `provider_id`, `model_id`, `provider_request_id`, `finish_reason`, `token_usage`, and `idempotency`. Its finish, usage, and idempotency values SHALL use Agent-owned closed projection types; it SHALL contain no prompt, tool arguments, provider body, headers, endpoint, credential, or raw error. `InvocationResult` SHALL include `model_evidence: InvocationModelEvidence`, populated by mapping the already-bounded gateway `GenerationEvidence`.
7. `LocalAgentRuntime::invoke`, `invoke_with_ceiling`, their internal generation path, Agent MCP `agent_runtime_invoke`, and all compatibility adapters SHALL propagate the same borrowed `InvocationControl` asynchronously without blocking or replacement.

## Workflow semantics and migration

1. `AgentInvoker::invoke`, `CeilingAgentRuntime::invoke_with_ceiling`, `WorkflowRunner::start`, `start_with_policy`, and the execution path SHALL use the exact boxed-Future signatures in [design.md](design.md). Workflow MCP `workflow_start` SHALL await that path directly; existing async MCP handlers SHALL not hide synchronous blocking.
2. Workflow SHALL continue to own attempt-stable downstream idempotency-key derivation, timeout-duration selection, one fixed absolute invocation deadline, active process-local cancellation registration, max-attempts-one/no-retry behavior, bounded evidence collection, replay suppression, CAS transitions, tenant isolation, and terminal reasons. It SHALL receive an object-safe `DeadlineFactory` through composition and use it only to obtain the wake-capable signal for that chosen absolute deadline.
3. Workflow SHALL pass one `InvocationControl` unchanged through its Agent adapter; Agent SHALL pass it unchanged to `LlmProvider`. Neither layer may mint a replacement key, replace or extend the deadline signal, or weaken cancellation.
4. Cancellation and deadline expiry SHALL each wake a pending invocation future. `workflow_cancel` may report local acknowledgement only when an active signal is registered. A cancellation/terminal-completion race SHALL publish exactly one legal terminal transition; late completion SHALL not overwrite cancellation.
5. Duplicate `workflow_start` for the same exact start identity SHALL return the existing run without another Agent or provider call. Conflicting key reuse SHALL fail before invocation. No automatic provider or Agent retry SHALL occur.
6. `PolicyAwareAgentInvoker` SHALL emit `InvocationEvidence::new("llm_generation", canonical_data)` before `InvocationEvidence::new("result", output)`. Workflow SHALL own the compact canonical safe encoding of `InvocationModelEvidence`; it SHALL encode only provider/model/request-id/finish/usage/idempotency, use fixed field names and closed normalized values, and include no raw provider data.
7. The `llm_generation` item SHALL satisfy the existing per-item `MAX_EVIDENCE_CHUNK_BYTES`, event-count, and run `max_evidence_bytes` ceilings using checked byte accounting. If it or the following result would exceed a ceiling, invocation SHALL fail with `LimitExceeded` and Workflow SHALL terminalize atomically; no partially collected evidence SHALL be persisted.

## Adapters and composition

1. Feature `static` SHALL expose a deterministic, standard-library-only `static` module. `StaticProvider` SHALL be configured with either `LlmError` or a validated request-independent `StaticFixture` containing response text, normalized tool calls, optional provider request ID, finish reason, token usage, and idempotency disposition—but no provider/model identity and no `GenerateResponse`. Every successful `generate(actual_request, ...)` SHALL call `GenerateResponse::new(actual_request, ...)`, so identity and declared-tool checks are relative to the actual request. The adapter SHALL perform no I/O, start no work, and deterministically exercise success, tool-call, error, cancellation, deadline, limit, and evidence behavior.
2. Feature `genai` SHALL be optional and default-disabled, expose only module `genai`, and be exactly `genai = ["dep:genai"]`. The workspace SHALL pin `genai = { version = "=0.6.0", default-features = false }`; the brick SHALL inherit it with `genai = { workspace = true, optional = true, default-features = false }`. There SHALL be no direct Tokio or `futures` dependency or feature mapping.
3. Immediately after minimal setup registers the renamed package, exact dependency, and feature—but before any adapter implementation—the graph SHALL compile on repository MSRV Rust 1.88 and be inspected with `cargo +1.88.0 check -p llm-gateway --no-default-features --features genai` and `cargo +1.88.0 tree -p llm-gateway --no-default-features --features genai -e features`. Setup MAY proceed under conditional Rust SME approval, but an MSRV conflict, unwanted default feature, or failed confinement check SHALL block all implementation and overall approval; it SHALL NOT be bypassed with a bespoke HTTP client.
4. The validator adapter mapping SHALL map only dependency `genai` to module/feature `genai`. `genai`'s transitive Tokio, reqwest, and futures crates SHALL be checked by default dependency isolation and the resolved feature tree; brick source SHALL not name them, and setup SHALL not register them as direct adapter mappings.
5. `genai` is selected because V1 needs maintained asynchronous multi-provider text generation beneath a narrow normalization adapter. Rig is rejected because its agents, tools, vector-store, memory, RAG, and orchestration surface overlaps Agent ownership. The archived `llm` repository is rejected because it is an unmaintained local-inference/CLI ecosystem with broad model/backend concerns rather than the required maintained provider client.
6. `GenaiProvider` SHALL receive an already configured `genai::Client` and validated non-secret `ProviderId`. It SHALL not discover credentials, read environment variables/files, select endpoints, create clients, install TLS roots, configure proxies, create a runtime or timer, own concurrency permits, spawn workers, or control shutdown. It SHALL poll the provider operation, cancellation future, and injected deadline future with standard-library future polling.
7. A composition root SHALL own credential and secret refresh, provider authentication resolution, endpoint allowlisting, DNS/network policy, TLS roots and verification policy, proxy policy, concrete client construction, runtime and runtime-backed `DeadlineFactory` selection/startup, concurrency limits, admission/backpressure, task ownership, and orderly shutdown. The adapter only executes one caller-polled operation using injected resources.
8. V1 SHALL have no `settings` feature/module because no composition binary consumes an adapter-selection schema, and no `mcp` feature/module because Agent and Workflow are the control planes.

## Non-goals and guarantees not made

Streaming or backpressure APIs; embeddings; image/audio; persisted chat history; routing, fallback, or caching; retries; direct LLM Gateway MCP; gateway-owned settings, credentials, endpoint selection, TLS/proxy policy, runtime, concurrency, or shutdown; durable state; background workers; remote abort; durable cancellation acknowledgement; exactly-once effects; provider-side recovery; mesh/distributed adapters; and process topology are out of scope.

## Quality and approval gates

1. The historical pre-implementation Rust SME gate conditionally approved crate ownership, exact APIs, object safety, limits, error/evidence taxonomy, async propagation, feature graph, and dependency choice for setup and implementation.
2. Adversarial QA covered exact-limit/one-over cases, checked-arithmetic overflow, malformed/non-object JSON, duplicate tool names, undeclared calls, oversized output/evidence, token inconsistency, cancellation/deadline races, dropped futures, replay suppression, tenant isolation, terminal-state races, and bounded broadcast cancellation, and recorded **APPROVE**.
3. Security review covered capability-scope derivation, trusted invocation context, reserved-name confinement, argument validation, pre-effect cancellation/deadline checks, endpoint/credential ownership, safe error/evidence/debug projection, idempotency honesty, egress ceilings, and absence of secret/provider-body leakage, and recorded **APPROVE**.
4. Final `meta-architect` and Rust SME gates confirmed inward dependencies, lifecycle ownership, contract fidelity, no hidden runtime, and clean future adapter seams. Both recorded **APPROVE**, with no Blocker or Required findings remaining after the core service, specification, and taxonomy corrections.
5. The focused default, `static`, `genai`, MCP-affected Agent/Workflow, and combined feature matrix in [tasks.md](tasks.md), followed by `make check`, passed on Rust 1.88.
6. Package status is promoted to `implemented`. Issue delivery/closure and any stable claim SHALL occur only after the remaining delivery evidence exists.

## Sources

- [Rust Factory issue #34: Build LLM Gateway capability and migrate Agent provider port](https://github.com/bannff/Rust-Factory/issues/34)
- [Rust Factory issue #8: Design async agent execution ports](https://github.com/bannff/Rust-Factory/issues/8)
- [rust-genai repository, v0.6.0](https://github.com/jeremychone/rust-genai/tree/v0.6.0)
- [rust-genai 0.6.0 API documentation](https://docs.rs/genai/0.6.0/genai/)
- [Rig documentation](https://rig.rs/docs/)
- [rustformers/llm repository](https://github.com/rustformers/llm)

External source content was rephrased for compliance with licensing restrictions.
