# Design: LLM Gateway

**Status:** implemented and validated; delivery remains pending
**Tracking:** [Issue #34](https://github.com/bannff/Rust-Factory/issues/34), with async-port prerequisite [Issue #8](https://github.com/bannff/Rust-Factory/issues/8)

## Implemented design evidence

The implementation follows this design across `crates/llm-gateway`, `crates/agent`, and `crates/workflow`. Request-relative response construction is centralized in the core `crates/llm-gateway/src/service.rs`; the `static` fixture and injected-client `genai` adapter call that core path rather than constructing unchecked evidence. `GenaiProvider` manually redacts `Debug`, exposing only the validated provider ID and omitting client/configuration state.

Agent now derives an explicit `InvocationContextV1` from trusted host/Policy context, keeps it out of gateway requests and scope digests, and passes it unchanged to Agent-owned effect ports; tests cover cross-tenant memory isolation. Workflow's process-local cancellation implementation is a bounded 64-subscriber broadcast with fail-closed capacity/token behavior, lost-wakeup coverage, and drop cleanup. Composition remains responsible for stable invocation-key construction, cancellation/deadline wake mechanics, configured clients, credentials, endpoint/egress policy, runtime, concurrency, task supervision, and lifecycle.

Adversarial QA, security, final Rust SME, and final meta-architect reviews recorded **APPROVE**. No Blocker or Required findings remain after the core service, specification, and taxonomy corrections. On Rust 1.88, the focused matrix passed with LLM Gateway default/static/genai totals of 18/20/30 tests, Agent default/MCP totals of 30/35, and Workflow default/memory/MCP+memory totals of 23/40/52. The Rust 1.88 `genai` check and feature-tree inspection and repository-wide `make check` also passed. Package status is promoted to `implemented`; delivery remains pending. This document does not claim stable status, merge, issue closure, or a deployable composition.

## Architecture and ownership

```text
Workflow (attempt lifecycle, deadline/key/cancellation, evidence, terminal CAS)
    │ AgentInvoker<'a> -> Future
    ▼
Agent (definition + capability ceiling + planning + reserved capability tools)
    │ LlmProvider<'a> -> Future
    ▼
llm-gateway core (bounded provider-neutral generation contract)
    ▲                         ▲
static module                 genai module
(no I/O)                      (injected configured client)
                                  ▲
composition root: credentials/endpoints/TLS/proxy/runtime/concurrency/shutdown
```

The rename is deliberately narrower than `model-gateway`: V1 normalizes bounded LLM text generation, not arbitrary model modalities. LLM Gateway owns provider-neutral generation mechanics. Agent owns why and under what capability scope a model is called. Workflow owns when an attempt runs and how its lifecycle is persisted. MCP remains on Agent and Workflow only.

## Exact public asynchronous API

The core uses only `std` future primitives. These signatures are normative:

```rust
use std::{future::Future, pin::Pin, time::Instant};

pub type ProviderFuture<'a> = Pin<
    Box<dyn Future<Output = Result<GenerateResponse, LlmError>> + Send + 'a>,
>;

pub type CancellationFuture<'a> =
    Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

pub type DeadlineFuture<'a> =
    Pin<Box<dyn Future<Output = ()> + Send + 'a>>;

pub trait CancellationSignal: Send + Sync {
    fn is_cancelled(&self) -> bool;
    fn cancelled(&self) -> CancellationFuture<'_>;
}

pub trait DeadlineSignal: Send + Sync {
    fn instant(&self) -> Instant;
    fn is_elapsed(&self) -> bool;
    fn elapsed(&self) -> DeadlineFuture<'_>;
}

pub trait DeadlineFactory: Send + Sync {
    fn create(&self, instant: Instant) -> Box<dyn DeadlineSignal>;
}

#[derive(Clone, Copy)]
pub struct InvocationControl<'a> {
    pub idempotency_key: &'a IdempotencyKey,
    pub cancellation: &'a dyn CancellationSignal,
    pub deadline: &'a dyn DeadlineSignal,
}

impl InvocationControl<'_> {
    pub fn preflight(&self) -> Result<(), LlmError>;
}

pub trait LlmProvider: Send + Sync {
    fn generate<'a>(
        &'a self,
        request: &'a GenerateRequest,
        control: InvocationControl<'a>,
    ) -> ProviderFuture<'a>;
}
```

`preflight` returns `Cancelled` if cancellation is already set, otherwise `DeadlineExceeded` if `deadline.is_elapsed()` is true; this ordering is deterministic when both are already true. `DeadlineSignal::instant()` exposes the fixed absolute `Instant` for request metadata and diagnostics without permitting an adapter to extend it. Adapters call `preflight` immediately before any network effect.

Workflow owns `INVOCATION_TIMEOUT`, computes the attempt's one absolute `Instant`, and calls the composition-injected `&dyn DeadlineFactory` once. The returned boxed signal remains owned by the execution future and is borrowed through `InvocationControl`. The factory owns only wake mechanics; the library does not create a timer, thread, executor, or runtime.

During I/O, `GenaiProvider` pins the provider operation, `control.cancellation.cancelled()`, and `control.deadline.elapsed()`, then races them through standard-library polling such as `std::future::poll_fn`. Each poll checks cancellation, then deadline, then the provider result, preserving deterministic tie handling without a direct Tokio or `futures` dependency. It does not turn the absolute deadline into a resettable per-step timeout.

Lifetime-only generic methods preserve object safety. Contract tests instantiate `&dyn LlmProvider`, `&dyn CancellationSignal`, `&dyn DeadlineSignal`, and `&dyn DeadlineFactory`. Borrowing avoids cloning potentially large requests and prevents detached `'static` work from being implied by the port.

## Exact core model

The following field shapes are normative; fields are private where a constructor must preserve validity.

```rust
pub struct ModelId(String);
pub struct ProviderId(String);
pub struct ToolName(String);
pub struct IdempotencyKey(String);
pub struct ProviderRequestId(String);

pub struct Prompt {
    system: Option<String>,
    input: String,
}

pub struct JsonObject {
    canonical: String,
}

pub struct ToolDefinition {
    name: ToolName,
    description: String,
    input_schema: JsonObject,
}

pub struct GenerationLimits {
    max_output_tokens: u32,
}

pub struct GenerateRequest {
    provider_id: ProviderId,
    model_id: ModelId,
    prompt: Prompt,
    tools: Vec<ToolDefinition>,
    limits: GenerationLimits,
}

pub struct ToolCall {
    name: ToolName,
    arguments: JsonObject,
}

pub struct TokenUsage {
    input_tokens: u32,
    output_tokens: u32,
    total_tokens: u32,
}

pub enum FinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Other,
}

pub enum IdempotencyDisposition {
    Unsupported,
    Accepted,
}

pub struct GenerationEvidence {
    provider_id: ProviderId,
    model_id: ModelId,
    provider_request_id: Option<ProviderRequestId>,
    finish_reason: FinishReason,
    token_usage: Option<TokenUsage>,
    idempotency: IdempotencyDisposition,
}

pub struct GenerateResponse {
    text: String,
    tool_calls: Vec<ToolCall>,
    evidence: GenerationEvidence,
}
```

Every type exposes read-only accessors and fallible constructors. `JsonObject::new` parses one JSON text, rejects duplicate object keys, requires an object root, recursively sorts keys, and stores a canonical compact representation. `ToolDefinition::new` requires exactly top-level `"type": "object"`; a missing, different, non-string, or otherwise ambiguous top-level `type` returns `Unsupported`. V1 performs no broader object-root inference and no full arbitrary JSON Schema execution.

`GenerateRequest::new` checks provider/model IDs, prompt limits, unique tool names, the exact schema rule, item limits, and checked aggregates. `GenerateResponse::new(request, text, tool_calls, provider_request_id, finish_reason, token_usage, idempotency)` derives provider/model identity from `request`, constructs normalized `GenerationEvidence`, and checks output, every tool name against that request's declarations, object arguments, item limits, checked aggregates, and token consistency. Adapters cannot provide independent provider/model evidence or construct an unchecked response.

### Hard ceilings

| Value | Ceiling |
|---|---:|
| Provider/model/idempotency/provider-request identifier | 256 bytes each |
| Tool name | 128 bytes |
| System text | 16 KiB |
| Input text | 16 KiB |
| System + input | 32 KiB checked aggregate |
| Tool definitions | 64 items |
| Tool description | 4 KiB each |
| Tool schema | 16 KiB each |
| Tool schemas | 64 KiB checked aggregate |
| Response text | 64 KiB |
| Tool calls | 64 items |
| Tool-call arguments | 16 KiB each |
| Tool-call arguments | 64 KiB checked aggregate |
| Requested output tokens | 1..=1,000,000 |
| Reported input tokens | 0..=1,000,000 |
| Reported output tokens | 0..=1,000,000 |
| Reported checked total | 0..=2,000,000 and equal to input + output |

All byte counts are UTF-8 byte lengths. Aggregate checks use `checked_add`; no validation uses a lossy cast or an accepting saturation.

## Error taxonomy

```rust
pub enum LlmError {
    InvalidRequest,
    LimitExceeded,
    Unsupported,
    Cancelled,
    DeadlineExceeded,
    Authentication,
    RateLimited,
    Unavailable,
    ProviderRejected,
    ProtocolViolation,
}
```

`InvalidRequest` covers caller-invalid core data. `LimitExceeded` covers a hard ceiling or arithmetic overflow. `Unsupported` covers a valid V1 request the selected adapter cannot represent. `Cancelled` and `DeadlineExceeded` report local observation, not provider acknowledgement. `Authentication`, `RateLimited`, `Unavailable`, and `ProviderRejected` normalize provider outcomes. `ProtocolViolation` covers malformed, inconsistent, undeclared, or unsafe provider output. Variants carry no raw provider strings. Adapter-private diagnostics may be logged only through composition-owned safe observability policy and are never an error source or public evidence payload.

## Agent migration

Agent deletes the superseded provisional provider contract instead of preserving aliases. It defines a safe projection independent of gateway internals:

```rust
pub enum InvocationModelFinishReason {
    Stop,
    Length,
    ToolCalls,
    ContentFilter,
    Other,
}

pub struct InvocationModelTokenUsage {
    pub input_tokens: u32,
    pub output_tokens: u32,
    pub total_tokens: u32,
}

pub enum InvocationModelIdempotency {
    Unsupported,
    Accepted,
}

pub struct InvocationModelEvidence {
    pub provider_id: String,
    pub model_id: String,
    pub provider_request_id: Option<String>,
    pub finish_reason: InvocationModelFinishReason,
    pub token_usage: Option<InvocationModelTokenUsage>,
    pub idempotency: InvocationModelIdempotency,
}

pub struct InvocationResult {
    pub capability_scope_digest: String,
    pub events: Vec<InvocationEvent>,
    pub output: String,
    pub model_evidence: InvocationModelEvidence,
}
```

These are the exact projection fields. Agent maps them from validated, bounded `GenerationEvidence`; it does not copy prompts, tool arguments, raw bodies, headers, endpoints, credentials, or raw errors. `InvocationContextV1` owns four distinct validated IDs: `TenantId`, `PrincipalId`, `RequestId`, and `CorrelationId`. `LocalAgentRuntime` receives that context by value before the agent ID, plus `&dyn LlmProvider` or a generic implementation, and returns a borrowed boxed future:

```rust
pub type InvocationFuture<'a> = Pin<
    Box<dyn Future<Output = Result<InvocationResult, DefinitionError>> + Send + 'a>,
>;

pub fn invoke<'a>(
    &'a self,
    context: InvocationContextV1,
    id: &'a AgentId,
    input: String,
    control: InvocationControl<'a>,
) -> InvocationFuture<'a>;

pub fn invoke_with_ceiling<'a>(
    &'a self,
    context: InvocationContextV1,
    id: &'a AgentId,
    input: String,
    ceiling: &'a EffectiveCapabilityCeilingV1,
    control: InvocationControl<'a>,
) -> InvocationFuture<'a>;
```

There is no context-free invocation overload. The runtime propagates the same context values to every Agent-owned `ToolRequest`, `MemoryRequest`, `KnowledgeRequest`, and `SandboxRequest`; adapters use that trusted context rather than deriving identity from model output or effect arguments. `InMemoryMemoryStore` keys values by `TenantId`, so recall and write are tenant-isolated within its process-local, non-durable store.

The internal future resolves one definition snapshot, validates input, intersects the effective ceiling, resolves allowed tool metadata, and builds one gateway request. The request contains provider/model identity, prompt, normalized allowed tool definitions, and generation limits only. Invocation context is excluded from the Gateway request, capability-scope digest, `InvocationModelEvidence`, and Workflow evidence. Neither the context nor other Agent or Policy structures cross the Gateway boundary.

Agent converts gateway errors into its safe `DefinitionError` taxonomy without embedding raw detail. Gateway `Cancelled`, `DeadlineExceeded`, and `LimitExceeded` remain distinguishable so Workflow can choose the correct terminal reason. Authentication, rate limiting, unavailability, rejection, unsupported behavior, and protocol violations map to safe stable Agent operation errors.

### Tool and capability compatibility

Agent assigns normalized gateway names to ordinary tools and maintains a reverse map to Agent-owned IDs. Returned names are resolved only through this map and checked again against the effective scope before invocation.

Agent also synthesizes these reserved definitions only when the effective scope permits them:

| Reserved name | Required object fields | Agent-owned result |
|---|---|---|
| `factory.memory.recall` | `query: string` | `MemoryRecall { query }` |
| `factory.memory.write` | `value: string` | `MemoryWrite { value }` |
| `factory.knowledge.search` | `query: string` | `KnowledgeSearch { query }` |
| `factory.sandbox.execute` | `action: string`, `arguments: string[]` | `SandboxExecute { action, arguments }` |

The existing Agent byte/item ceilings still apply after JSON decoding. Unknown fields, wrong types, absent required fields, and extra items fail before a capability adapter call. Reserved names cannot be registered by users or returned as ordinary tools. LLM Gateway knows none of these semantics.

Agent MCP changes only the implementation path: `invoke_json` becomes async and `agent_runtime_invoke` awaits it. Definition validation/get/list/register remain synchronous internally and retain their current policy-before-domain behavior.

## Workflow migration

The Workflow port becomes:

```rust
pub type AgentInvocationFuture<'a> = Pin<
    Box<dyn Future<Output = Result<AgentInvocationResult, WorkflowError>> + Send + 'a>,
>;

pub trait InvocationEvidenceSink: Send {
    fn emit(&mut self, evidence: InvocationEvidence) -> Result<(), WorkflowError>;
}

pub trait AgentInvoker: Send + Sync {
    fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError>;

    fn invoke<'a>(
        &'a self,
        request: AgentInvocationRequest,
        control: InvocationControl<'a>,
        evidence: &'a mut dyn InvocationEvidenceSink,
    ) -> AgentInvocationFuture<'a>;
}
```

`AgentInvocationRequest` retains trusted Workflow/Agent data (`RequestContext`, `AgentId`, input, attempt ID, effective ceiling, and policy-decision digest) but removes `downstream_idempotency_key`, `cancellation`, and `deadline`; those exist exactly once in `InvocationControl`.

The MCP compatibility seam becomes:

```rust
pub type CeilingInvocationFuture<'a> = Pin<
    Box<dyn Future<Output = Result<agent::InvocationResult, WorkflowError>> + Send + 'a>,
>;

pub trait CeilingAgentRuntime: Send + Sync {
    fn validate_agent(&self, id: &AgentId) -> Result<bool, WorkflowError>;

    fn invoke_with_ceiling<'a>(
        &'a self,
        invocation: CeilingAgentInvocation,
        control: InvocationControl<'a>,
    ) -> CeilingInvocationFuture<'a>;
}
```

`CeilingAgentInvocation` explicitly includes Agent `InvocationContextV1`, converted losslessly from Workflow `RequestContext` by validating and preserving its tenant, principal, request, and correlation ID values. It removes cancellation, deadline, and idempotency fields. `PolicyAwareAgentInvoker` validates attempt-bound policy evidence before constructing or polling the runtime future, forwards the converted invocation context and `InvocationControl` unchanged, awaits the result, and handles evidence in this exact order:

1. Convert `result.model_evidence` into compact canonical JSON owned by Workflow.
2. Emit `InvocationEvidence::new("llm_generation", canonical_data)`.
3. Emit `InvocationEvidence::new("result", result.output)`.
4. Return the capability-scope digest.

The canonical `llm_generation` object contains exactly `finish_reason`, `idempotency`, `model_id`, `provider_id`, `provider_request_id`, and `token_usage`, in that lexicographic key order, with no whitespace. `provider_request_id` and `token_usage` are JSON `null` when absent. Closed enum strings are `stop`, `length`, `tool_calls`, `content_filter`, `other`, `unsupported`, and `accepted`; a present usage object contains exactly `input_tokens`, `output_tokens`, and `total_tokens` in that order. Workflow escapes strings with its canonical JSON encoder and never admits any other field or raw provider value.

`InvocationEvidence::new` applies the existing `MAX_EVIDENCE_CHUNK_BYTES` limit to `kind.len() + data.len()`. The collector uses checked addition for the run's `max_evidence_bytes` and event-count ceilings. If `llm_generation` or `result` exceeds any ceiling, the invocation returns `LimitExceeded`; the execution path atomically terminalizes failure and persists none of the collector's partial items.

`WorkflowRunner` receives `&dyn DeadlineFactory` (or owns an injected implementation) at construction. `start` and `start_with_policy` return `WorkflowFuture<'a, Result<RunSummary, WorkflowError>>`, where `WorkflowFuture<'a, T> = Pin<Box<dyn Future<Output = T> + Send + 'a>>`. The private execution future chooses `INVOCATION_TIMEOUT`, computes one absolute `Instant`, asks the factory once for its boxed `DeadlineSignal`, registers cancellation, and borrows the key, cancellation, and deadline signals into one `InvocationControl` before polling the invoker. Read/list and cancellation remain synchronous because they perform only bounded local port operations. `workflow_start` awaits the runner. Other MCP handlers retain their existing async signatures.

Workflow's concrete process-local cancellation handle implements `llm_gateway::CancellationSignal` with an atomic flag plus registered wakers. `cancel()` sets the flag and wakes all registered waiters. Poll registration rechecks the flag after registering to close the lost-wakeup race. The implementation starts no executor and is runtime-neutral. Workflow does not implement deadline wake mechanics; the composition-injected factory returns the runtime-backed signal.

A guard unregisters the active cancellation entry on every completion, error, or dropped execution future. Completion, cancellation, and deadline expiry compete through the existing store CAS; a conflict reads the already-published terminal state. Evidence-limit errors terminalize as failure atomically. Duplicate starts do not poll another invocation.

## Adapter design

### `static`

`static` is default-disabled and std-only. `StaticProvider` is configured with `Result<StaticFixture, LlmError>`, never with `GenerateResponse`. A validated fixture is request-independent:

```rust
pub struct StaticFixture {
    text: String,
    tool_calls: Vec<ToolCall>,
    provider_request_id: Option<ProviderRequestId>,
    finish_reason: FinishReason,
    token_usage: Option<TokenUsage>,
    idempotency: IdempotencyDisposition,
}
```

`StaticFixture::new` validates its raw normalized text, calls, and evidence inputs where request-independent validation is possible, but stores no provider/model identity and performs no declared-tool membership check. On every successful `generate(actual_request, control)`, `StaticProvider` runs `control.preflight()` and calls `GenerateResponse::new(actual_request, fixture.text.clone(), fixture.tool_calls.clone(), fixture.provider_request_id.clone(), fixture.finish_reason, fixture.token_usage.clone(), fixture.idempotency)`. Consequently provider/model identity is derived from the actual request and tool declarations are checked against that request on every call. The adapter performs no I/O, spawning, sleeping, timer creation, or mutation.

### `genai`

The manifest decision is exact:

```toml
[features]
static = []
genai = ["dep:genai"]

[dependencies]
genai = { workspace = true, optional = true, default-features = false }
```

The workspace pins `genai = { version = "=0.6.0", default-features = false }`; the brick inherits that exact dependency with `workspace = true`, `optional = true`, and `default-features = false`. There is no direct Tokio or `futures` dependency or feature mapping. `genai` 0.6.0's normal dependency graph includes Tokio, futures, reqwest, streaming utilities, and provider support transitively. Setup maps only direct dependency `genai` to module/feature `genai`; the default dependency-isolation check and resolved feature-tree vet cover those transitive crates, and `llm-gateway` source does not name them.

Immediately after minimal setup registers the renamed package and this exact feature/dependency—but before any adapter implementation—the conditional approval gate runs:

```sh
cargo +1.88.0 check -p llm-gateway --no-default-features --features genai
cargo +1.88.0 tree -p llm-gateway --no-default-features --features genai -e features
```

Setup may proceed under conditional Rust SME approval. An MSRV failure, unwanted default feature, or failed confinement check blocks all implementation and overall approval; it is not bypassed with a bespoke HTTP client.

`GenaiProvider::new(client: genai::Client, provider_id: ProviderId)` accepts a preconfigured client. The adapter translates validated requests and initiates one client future. It pins that operation, the borrowed cancellation future, and the borrowed injected deadline future, then uses standard-library polling such as `std::future::poll_fn` to resolve whichever becomes ready first; each poll checks cancellation, deadline, then provider completion. It creates no timer or runtime and names no runtime type. It normalizes raw response inputs and calls `GenerateResponse::new(actual_request, ...)`, without exposing `genai` DTOs or errors.

Rig is not selected because its documented agent, tool, vector-store, memory, RAG, and orchestration abstractions would overlap Agent and Workflow ownership. The rustformers `llm` project is archived and centers a local GGML model/backend and CLI ecosystem. `genai` is the narrower maintained provider client for this demonstrated requirement, subject to the early MSRV/feature-graph gate.

## Composition boundary

Only a future `projects/` binary may obtain/refresh secrets, resolve authentication, allow endpoints, choose DNS/network confinement, install TLS roots, set certificate verification, configure proxies, construct `genai::Client`, select/start the async runtime, construct the runtime-backed `DeadlineFactory`, own concurrency semaphores and admission queues, supervise tasks, and coordinate shutdown. Workflow owns the timeout duration and chosen absolute deadline; the injected factory supplies only wake mechanics for that `Instant`. The adapter receives constructed resources and borrowed control signals, returns caller-polled futures, and cannot promise orderly process shutdown or provider cancellation acknowledgement.

No `settings` module exists until a composition consumer proves a schema. No LLM Gateway MCP exists because Agent and Workflow expose the bounded operational surfaces.

## Verification design

- Core: constructor/property tests at every exact limit and one over; duplicate JSON keys; non-object JSON; exact top-level `"type": "object"` schema rule; duplicate/undeclared tools; checked overflow; token consistency; safe errors/evidence; trait-object compilation for provider, cancellation, deadline, and factory ports.
- Static: request-independent fixture validation; request-relative provider/model identity and declared-tool checks on every call; deterministic success/tool/error; cancellation and deadline before first effect; dropped future; no I/O/runtime dependency.
- Agent: ceiling intersection before request construction; reserved mapping and collision rejection; malformed arguments; output/tool limits; exact `InvocationModelEvidence` projection; unchanged invocation control; safe error mapping.
- Workflow: injected deadline factory receives the Workflow-chosen absolute `Instant`; duplicate replay and key conflict; no retry; cancellation/deadline wake pending work; cancellation/completion CAS race; dropped-start cleanup; canonical `llm_generation` before `result`; per-item/aggregate evidence-limit terminalization without partial persistence; tenant isolation.
- MCP: async start/invoke await paths, policy-before-effect ordering, bounded response/error projection, and no identity/provider-secret fields.
- Dependency: default/static isolation; exact `genai = ["dep:genai"]` and pin; immediate Rust 1.88 build and resolved feature-tree vet after minimal setup; only `genai` mapped to `genai`; no Tokio/futures name in brick source; and no timer/runtime creation in library code.

## Sources

- [Rust Factory issue #34](https://github.com/bannff/Rust-Factory/issues/34)
- [Rust Factory issue #8](https://github.com/bannff/Rust-Factory/issues/8)
- [rust-genai v0.6.0 repository](https://github.com/jeremychone/rust-genai/tree/v0.6.0)
- [rust-genai 0.6.0 API documentation](https://docs.rs/genai/0.6.0/genai/)
- [Rig documentation](https://rig.rs/docs/)
- [rustformers/llm repository](https://github.com/rustformers/llm)

External source content was rephrased for compliance with licensing restrictions.
