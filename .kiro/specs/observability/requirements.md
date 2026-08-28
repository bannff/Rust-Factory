# Observability Brick

Bounded, tenant-scoped operational log telemetry behind framework-agnostic ports, with process-local inspection and metadata-only OpenTelemetry submission as opt-in adapters. The `observability` family owns the `observability` crate. GitHub [Issue #19](https://github.com/bannff/Rust-Factory/issues/19) tracks this V1 delivery.

## 1. Capability and scope

1. V1 SHALL carry **operational logs only**. `TelemetryEventV1` represents a severity-bearing event with a body and bounded string attributes; the brick does not define spans, traces, metrics, or audit records.
2. Operational telemetry SHALL NOT be presented as an audit trail. The current adapters provide no immutable history, retention, completeness, tamper evidence, durable delivery, or durable reader guarantee.
3. Operational telemetry SHALL remain distinct from Workflow lifecycle evidence and Evaluation acceptance evidence. Workflow owns attempt state, budgets, cancellation, terminal reasons, and execution evidence; Evaluation independently assesses terminal evidence. An Observability event cannot substitute for either contract.
4. The framework-free core SHALL own typed models, validation, errors, `TelemetrySink`, `TelemetryReader`, `Clock`, and `TelemetryService`. Local storage, OpenTelemetry, settings DTOs, and MCP SHALL remain feature-gated adapter modules. No feature is enabled by default.

## 2. Typed model, limits, and grammar

All ceilings are brick-owned constants and count UTF-8 bytes where a byte limit applies:

| Contract | V1 limit |
|---|---:|
| Tenant ID, event name, event target, attribute key | 128 bytes |
| Body | 16 KiB |
| Attributes per event | 32 |
| Attribute value | 1 KiB |
| Aggregate event name + target + body + attribute keys and values | 64 KiB |
| Query result request | 256 records |
| Local records per tenant | 4,096 |
| Local records across all tenants | 4,096 |

1. `TenantId` SHALL be non-empty, begin with an ASCII lowercase letter or digit, and continue with only ASCII lowercase letters, digits, `_`, or `-`.
2. `EventName`, `EventTarget`, and attribute keys SHALL be non-empty, begin with an ASCII lowercase letter or digit, and continue with only ASCII lowercase letters, digits, `_`, `-`, or `.`. Constructors enforce event-name and target grammar; `validate_event` enforces attribute-key grammar.
3. `TelemetryEventV1` SHALL use the closed severity order `Trace < Debug < Info < Warn < Error`. Event constructors and every current sink revalidate the event rather than inheriting provider limits.
4. `TelemetryEnvelopeV1` SHALL bind one validated tenant, one unsigned Unix-nanosecond timestamp, and one event. A valid `TelemetryRecordV1` SHALL have a non-zero sequence.
5. `TelemetryQueryV1` SHALL support optional `since`, `until`, minimum severity, exact event name, and exact target filters. Filters are conjunctive; `since` is inclusive and `until` is exclusive. When both bounds exist, `since` SHALL be less than `until`.
6. Query limits SHALL be positive and no greater than both `MAX_QUERY_LIMIT` and the deployment ceiling passed to `TelemetryService::new`. Limits count matching records, not records examined.
7. `MAX_EVENT_BYTES` is a defensive aggregate ceiling. The current individual V1 ceilings make the maximum reachable event smaller than 64 KiB; this SHALL NOT be described as permission to increase an individual field without a separate contract change.

## 3. Ports, errors, and truthful guarantees

1. `TelemetrySink`, `TelemetryReader`, and `Clock` SHALL remain synchronous, object-safe, `Send + Sync` ports. `Arc<dyn TelemetrySink>`, `Arc<dyn TelemetryReader>`, and `Arc<dyn Clock>` are supported so a composition root can select concrete implementations at startup.
2. A sink accepts a validated, tenant-stamped `TelemetryEnvelopeV1`. A reader accepts a tenant separately from `TelemetryQueryV1`; the query has no tenant field. A clock supplies the timestamp used by the service rather than accepting caller time on emit.
3. `TelemetryGuarantees` SHALL report four independent facts: `durable_across_restart`, `visible_across_processes`, `delivery_confirmed`, and `queryable`. `false` means the guarantee is not provided; callers SHALL NOT infer a stronger provider-specific property.
4. `TelemetryService::guarantees` SHALL report durability and cross-process visibility only when both sink and reader report them, sink delivery confirmation directly, and reader queryability directly. A sink and reader may be different adapters; status describes their composition.
5. The stable error taxonomy SHALL expose invalid ID, event, query, configuration, limit, and operation-failed classes. Adapter failures collapse to `OperationFailed`; `Display` and `Debug` SHALL NOT reveal backend text, paths, provider details, or secrets.
6. The ports do not claim retries, acknowledgement by an exporter, exactly-once delivery, recovery, leases, retention, or crash atomicity. Those properties require a separately specified adapter and, where applicable, lifecycle owner.

## 4. Service tenant and time scoping

1. `TelemetryService::emit` SHALL derive the envelope tenant from `TelemetryContext` and the timestamp from the injected `Clock`. Neither value is accepted in `TelemetryEventV1`, so an event caller cannot widen tenant scope or choose service time through the event payload.
2. `TelemetryService::query` SHALL validate the query against the deployment ceiling, call the reader with the context tenant, discard invalid records, discard records for other tenants, reapply every query filter, and truncate to the requested matching-record limit. This defence remains required even when a reader claims to enforce the same contract.
3. `TelemetryContext::new` and `TenantId::new` are public. Any code that can name a tenant can mint a context, so context construction is privileged composition work rather than authentication performed by this brick.
4. MCP context SHALL be derived from an injected host `TrustedContextSource` and verified Policy decision, never from request identity or grant fields. A future binary owns construction and distribution of that source.

## 5. Local adapter

`local::LocalTelemetry` is one shared in-process sink and reader protected by a mutex.

1. Construction SHALL accept a per-tenant capacity from 1 through `MAX_LOCAL_EVENTS_PER_TENANT`.
2. Records SHALL receive a non-zero, globally increasing sequence at accepted insertion. Queries return newest insertion/sequence first; caller timestamps do not define storage order.
3. Each tenant SHALL have an independent FIFO partition. When that partition is full, a successful emit evicts its oldest record before retaining the new record.
4. The store SHALL hold at most `MAX_LOCAL_EVENTS_TOTAL` records across all tenants. At the global limit, an emit that would grow the total SHALL fail with `LimitExceeded`; an emit to an already full tenant may still replace its oldest record because total cardinality does not grow.
5. Reads SHALL be tenant-isolated, honor every filter, and apply the limit after filtering. The adapter SHALL revalidate envelopes and queries at its own public boundary.
6. Poisoned state SHALL fail closed as `AdapterFailure` for both reads and writes.
7. Its truthful guarantees are: not durable across restart, not visible across processes, delivery confirmed only as acceptance into this in-memory store, and queryable. Eviction is expected and means the adapter is not an evidence or retention store.

## 6. Settings boundary

The `settings` feature owns the closed Serde/Schemars DTO and semantic conversion, not configuration I/O or adapter construction. The accepted V1 TOML shapes are:

```toml
version = 1
max_query_limit = 32

[sink]
type = "local"
max_events_per_tenant = 128
```

or:

```toml
version = 1
max_query_limit = 8

[sink]
type = "open_telemetry_logs"
```

1. `ObservabilityConfigV1` and its tagged sink enum SHALL reject unknown fields and unknown variants. Only `version = 1` is valid.
2. `max_query_limit` SHALL be in `1..=MAX_QUERY_LIMIT`; local capacity SHALL be in `1..=MAX_LOCAL_EVENTS_PER_TENANT`.
3. `ObservabilitySettings` is validated data only. A composition binary SHALL own the source of TOML or equivalent configuration, the sink-to-constructor match, reader and clock selection, feature availability errors, Policy and trusted-context construction, and shutdown.
4. No composition binary exists, so declarative selection is contract-tested at decode/conversion boundaries but is not proven end to end in a runnable process.

## 7. OpenTelemetry logs adapter

The `opentelemetry` feature contains an API-level logs sink over exact-pinned `opentelemetry =0.32.0` with `default-features = false` and `features = ["logs"]`.

1. `OpenTelemetryLogsSink` SHALL receive an injected `opentelemetry::logs::Logger`. It SHALL NOT construct an SDK, exporter, runtime, batch processor, network client, or shutdown hook; those belong to a composition binary.
2. It SHALL call `event_enabled` before creating a record. A disabled event is accepted without record creation or emission.
3. Egress SHALL contain only fixed OpenTelemetry event name `rust_factory.telemetry`, fixed target `rust_factory.observability`, event timestamp, mapped severity number/text, and the validated source event name and target under `rust_factory.event_name` and `rust_factory.event_target`.
4. It SHALL NOT export tenant ID, body, caller attributes, or caller-supplied `rust_factory.*` values. This metadata-only projection is the V1 data-minimization boundary.
5. Its guarantees are all false: API submission does not prove restart durability, cross-process visibility, exporter delivery, or queryability.

## 8. Policy-gated MCP adapter

The `mcp` feature exposes exactly two read/status tools:

| Tool | Policy capability | Result |
|---|---|---|
| `observability_telemetry_query` | `ObservabilityTelemetryQuery` | Bounded tenant-scoped records |
| `observability_telemetry_status` | `ObservabilityTelemetryStatus` | Composed adapter guarantees |

1. Both request schemas SHALL be closed and SHALL contain no tenant, principal, request, correlation, grant, decision-digest, or policy field. The status input is empty.
2. Serialized request DTOs SHALL be at most 16 KiB and SHALL be checked before trusted-context, Policy, reader, or other domain effects, leaving conservative headroom inside the transport's complete inbound JSON-RPC frame. Semantic query conversion and validation SHALL run only after authorization so a denied caller cannot probe field validity. The composition root still owns and tests the complete inbound envelope and request-ID bound.
3. The resolver SHALL obtain trusted context from the host source, authorize the exact capability, canonicalize the effective grant, recompute the request-bound decision digest, reject deny or tampering, and derive the telemetry tenant from trusted context.
4. Query output SHALL contain only sequence, Unix-nanosecond timestamp, severity, event name, target, and an explicit `truncated` boolean. It SHALL omit tenant, body, and attributes. Raw serialized output SHALL remain within a conservative tool-text budget, and its measured JSON-string-escaped tool text SHALL stay below its own bound; if projection omits records, `truncated` SHALL be true so a caller cannot mistake a partial view for a complete one. These brick-local checks do not bound the complete JSON-RPC envelope because the echoed request ID is composition-owned and unbounded here; the first server binary must enforce and test the full outbound frame.
5. Status output SHALL contain only the four `TelemetryGuarantees` booleans and SHALL not call the reader.
6. Public failures SHALL project only the stable snake-case codes. Source failure, Policy denial, digest tampering, reader failure, and unexpected serialization detail SHALL not leak internal text.
7. The MCP surface is read-only. It SHALL NOT expose event emission, configuration mutation, exporter control, or lifecycle operations.

## 9. Test strategy and checked evidence

- `tests/public_contract.rs` — framework-free identifier/label grammar, byte/count limits, half-open query semantics, stable non-leaking errors, tenant/clock stamping, hostile-reader filtering, deployment ceilings, guarantees composition, and object-safe trait use.
- `tests/local_contract.rs` plus the local module test — adapter ingress/query validation, tenant isolation, deterministic sequence/order/eviction, conjunctive filtering, concurrent writers, global cardinality, and poisoned-state failure.
- `tests/settings_contract.rs` — closed Serde/Schemars shapes, semantic limits, stable backend names, and both exact TOML forms.
- `tests/opentelemetry_contract.rs` — all severity mappings, metadata-only egress, reserved-attribute resistance, disabled events, and submit-only guarantees.
- `src/mcp.rs` contract tests — exact tool/schema surface, byte-exact request ceiling before authorization, semantic validation only after authorization, Policy deny/tamper/source failures before reader effects, trusted tenant derivation, exact capabilities, raw and escaped/framed safe record/status projection, and error redaction.
- `policy` tests bind both Observability capability wire names into Policy decision digests.

The narrow delivery command is `cargo test -p observability --all-features`. The final workspace acceptance command remains `make check` and is not evidence until it completes successfully.

## 10. Named gaps

None of these is claimed as done:

1. **No Agent or Workflow integration.** Neither emits through this brick; no autonomous runtime path demonstrates operational telemetry end to end.
2. **No durable reader or exporter.** Local records disappear with the process and evict at bounded capacity. The OpenTelemetry module accepts only an API logger and proves neither exporter construction nor exporter delivery.
3. **No spans, traces, metrics, or audit.** V1 is logs-only operational telemetry and cannot replace Workflow/Evaluation evidence.
4. **No composition binary.** Settings source, adapter construction, trusted context, Policy composition, OpenTelemetry SDK/exporter/runtime, transport binding, and orderly flush/shutdown are unimplemented.
5. **No `cargo-audit` gate.** `make check` has no advisory scan, so the exact OpenTelemetry pin has no repository-enforced advisory monitoring.
6. **No durable retention or recovery contract.** Retention periods, retries, acknowledgement, replay, crash recovery, and cross-process querying require separately specified adapters and lifecycle ownership.
