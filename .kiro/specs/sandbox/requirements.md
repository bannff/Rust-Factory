# Sandbox Brick

`crates/sandbox` owns the `sandbox` family: a minimal, provider-neutral sandbox capability (start/execute/status/stop) with optional Docker CLI, MCP, and observability-event adapters. The crate is `status = "migration-pending"`; this document does not attempt a full retroactive specification of the whole brick, only the contract introduced by GitHub [Issue #58](https://github.com/bannff/Rust-Factory/issues/58) (the `observability` adapter), whose design decisions were resolved by rust-factory-sme review and are recorded here so they are not relitigated or accidentally violated by a future change.

## 1. `observability` adapter (`src/observability.rs`, feature-gated)

`TelemetryEventSink<S, C>` implements `SandboxEventSink` over an injected `S: observability::TelemetrySink` and `C: observability::Clock`. It is the only bridge from Sandbox's core event port to Observability; `crates/observability` itself is unmodified by this adapter (dependency flows inward: sandbox depends on observability's port/model types, never the reverse).

1. `SandboxEventSink::try_emit` SHALL remain synchronous, non-blocking, and best-effort, per its existing doc contract ("must not perform network or disk I/O" at the call site). This adapter SHALL read the injected sink's `TelemetryGuarantees::may_block` exactly once, at construction (`TelemetryEventSink::new`), and store it. It SHALL NOT re-read `guarantees()` on any subsequent `try_emit` call: `may_block` is a static property of the sink, not a per-invocation check.
2. If the construction-time `may_block` snapshot is `true`, every `try_emit` call SHALL return `EventSubmission::Dropped` immediately, before any tenant conversion, clock call, or attribute construction, and SHALL NOT call the injected sink's `emit` at all.
3. `sandbox::CorrelationId` SHALL be forwarded only as a plain string value under a `"correlation_id"` key in `TelemetryEventV1.attributes`. It SHALL NOT be mapped onto `observability::TraceContextV1` or any trace-context-shaped field: the two concepts are related but distinct, and this adapter does not conflate them.
4. `sandbox_id` and `status` (when `Some`) SHALL become plain attribute strings (`"sandbox_id"`, `"status"`). When absent (`None`), the corresponding key SHALL be omitted entirely; the adapter SHALL NOT emit an empty-string placeholder.
5. `sandbox::TenantId` SHALL be converted to `observability::TenantId` via `observability::TenantId::new(sandbox_tenant.as_str())` at every `try_emit` call (not cached). Conversion failure — including a hypothetical future grammar divergence between the two independently-validated newtypes — SHALL result in `EventSubmission::Dropped`, never a panic and never emission under a wrong or default tenant.
6. A sink `emit` call that returns `Err` SHALL be reported as `EventSubmission::Dropped`, never propagated or panicked on. `try_emit`'s return type carries no error channel, so this is the only observable outcome for any adapter-side or sink-side failure.
7. The adapter SHALL NOT perform any authorization, capability, or trust decision; it purely maps and forwards a `SandboxEvent` that has already passed through `SandboxService`'s existing, unmodified authorization/validation path.

## 2. Cargo and feature shape

1. `observability` SHALL be an optional, feature-gated dependency (`observability = ["dep:observability"]`), off by default, with no forwarded sub-features in `[dependencies]`: sandbox is a leaf brick, not a composition root, so it SHALL NOT choose which concrete `observability` backend (`local`, `opentelemetry`, ...) is compiled in. That choice belongs to whichever composition binary injects the concrete sink.
2. A `[dev-dependencies]` entry enabling `observability`'s `local` feature is permitted for this adapter's own tests only, matching existing workspace precedent for dev-only feature forwarding on a path dependency.
3. The module SHALL be declared `#[cfg(feature = "observability")] pub mod observability;` in `lib.rs`, the only feature-gated item at that location (adapter-module-only gating, per the Canonical Brick Standard).

## 3. Test strategy

`src/observability.rs`'s own `#[cfg(test)]` module covers: the `may_block` construction-time gate (including a toggling-guarantees regression test proving the value is never re-read), correlation-id/trace-context non-conflation, present/absent `sandbox_id`/`status` attribute mapping, sink `Err` mapping to `Dropped`, concurrent `try_emit` calls from multiple threads sharing one adapter instance, every `SandboxOperation` producing a grammar-valid `EventName`, and a regression guard that `sandbox::TenantId` and `observability::TenantId` currently accept the same inputs.
