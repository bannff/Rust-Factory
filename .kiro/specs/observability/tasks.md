# Tasks: Observability

- [x] Define the logs-only V1 model, identifier/label grammar, byte/count ceilings, half-open query semantics, stable error projection, and truthful guarantee flags.
- [x] Define object-safe `TelemetrySink`, `TelemetryReader`, and `Clock` ports plus `TelemetryService` tenant/time stamping, deployment query ceiling, hostile-reader filtering, and guarantees composition.
- [x] Implement the bounded `local` sink/reader with per-tenant FIFO eviction, a 4,096-record global ceiling, deterministic sequence ordering, tenant isolation, concurrent-writer safety, and process-local guarantees.
- [x] Implement closed Serde/Schemars `settings` DTOs and semantic conversion for the exact V1 local/OpenTelemetry logs TOML shapes, without moving source or construction ownership into the brick.
- [x] Add the exact-pinned OpenTelemetry 0.32.0 API logs adapter with injected logger, metadata-only egress, disabled-event short circuit, and no SDK/exporter/runtime/shutdown ownership.
- [x] Add Policy capabilities and exactly two read/status MCP tools with host-derived tenant context, request-bound decision verification, closed schemas, a conservative 16 KiB request-DTO ceiling, escaped tool-text response ceilings, explicit partiality, and whole-record truncation.
- [x] Add focused framework-free, local, settings, OpenTelemetry, MCP, and Policy contract tests for limits, tenant isolation, eviction, concurrency, configuration, metadata minimization, authorization ordering, and error redaction.
- [x] Reconcile README and Living Factory Vision wording with the implemented logs-only, process-local/API-submission contract and record the named V1 gaps.
- [x] Run final `make check`; the recovered combined workspace passed after focused recovery QA, security, final Rust SME, and final architecture approval.

## Acceptance evidence

- `cargo test -p observability --all-features` passed during recovery: 29 unit/integration tests plus doc tests, covering the framework-free core and every current adapter feature.
- Public contract tests prove core limits, service scoping, safe errors, hostile-reader defence, and object-safe composition.
- Adapter tests prove local bounds/eviction/concurrency, closed TOML settings, and metadata-only OpenTelemetry API submission.
- MCP tests prove exact read/status tools, trusted-context and Policy ordering, decision-digest verification, bounded safe projections with explicit partiality, and no caller-supplied identity/policy fields.
- `cargo test -p policy` passed during recovery: 10 tests, including Memory and Observability capability wire-name/digest coverage.
- Historical task history records the original issue #19 reviews. Current recovery evidence is explicit: `qa-tester` APPROVE, `security-reviewer` APPROVE, final `rust-factory-sme` APPROVE, final `meta-architect` APPROVE, followed by a successful full `make check`.
- Final workspace acceptance passed on the recovered combined tree.

No Agent/Workflow integration, durable reader/exporter, spans/traces/metrics/audit, composition binary, or `cargo-audit` gate is delivered by this checklist.
