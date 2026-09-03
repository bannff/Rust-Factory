# Observability Brick

Bounded, tenant-scoped operational log, span, and metric telemetry behind framework-agnostic ports, with process-local inspection, W3C Trace Context propagation, and metadata-only OpenTelemetry submission as opt-in adapters. The `observability` family owns the `observability` crate. GitHub [Issue #19](https://github.com/bannff/Rust-Factory/issues/19) tracked the original V1 (logs-only) delivery; [Issue #44](https://github.com/bannff/Rust-Factory/issues/44) and its sub-issues [#56](https://github.com/bannff/Rust-Factory/issues/56)/[#57](https://github.com/bannff/Rust-Factory/issues/57) track the span/metric/propagation expansion.

## 1. Capability and scope

1. V1 SHALL carry **operational logs, spans, and metrics**. `TelemetryEventV1` represents a severity-bearing log event with a body and bounded string attributes. `SpanEventV1` represents one completed span bound to a W3C `TraceContextV1`. `MetricEventV1` represents one numeric data point (counter, gauge, or histogram). None of these are audit records.
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
3. `TelemetryGuarantees` SHALL report five independent facts: `durable_across_restart`, `visible_across_processes`, `delivery_confirmed`, `queryable`, and `may_block`. `false` means the guarantee is not provided (or, for `may_block`, that `emit` has been verified not to block on I/O); callers SHALL NOT infer a stronger provider-specific property. An adapter that cannot verify whether an injected downstream dependency blocks on I/O SHALL report `may_block: true` rather than an unverifiable `false`.
4. `TelemetryService::guarantees` SHALL report durability and cross-process visibility only when both sink and reader report them, sink delivery confirmation directly, and reader queryability directly. `may_block` SHALL mirror the sink's value only: it is an emit-path-only concern and the reader's blocking behavior SHALL NOT be composed into it. A sink and reader may be different adapters; status describes their composition.
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
7. Its truthful guarantees are: not durable across restart, not visible across processes, delivery confirmed only as acceptance into this in-memory store, queryable, and not `may_block` (a `Mutex`-guarded `VecDeque` with no network or disk I/O). Eviction is expected and means the adapter is not an evidence or retention store.

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
5. `durable_across_restart`, `visible_across_processes`, `delivery_confirmed`, and `queryable` are all false: API submission does not prove restart durability, cross-process visibility, exporter delivery, or queryability. `may_block` SHALL report `true`: this adapter wraps an injected `Logger` whose underlying exporter I/O behavior it cannot construct, observe, or bound, and an unverifiable `false` claim is not permitted (§3.3).

## 7a. Trace/metric core model and W3C propagation

Additive to the log-only model above; none of it changes `TelemetryEventV1`, `TelemetrySink`, `TelemetryReader`, or their existing tests.

1. `TraceId` (16 bytes) and `SpanId` (8 bytes) SHALL reject an all-zero value at construction. `TraceFlags` preserves the raw W3C trace-flags byte and exposes only the currently-defined `sampled` bit.
2. `TraceContextV1` binds a `TraceId`, `SpanId`, `TraceFlags`, an optional opaque `trace_state` (bounded by `MAX_TRACESTATE_BYTES`), and bounded `baggage` (at most `MAX_BAGGAGE_ENTRIES` entries, each value at most `MAX_BAGGAGE_VALUE_BYTES`). `trace_state` and baggage values SHALL reject C0 control bytes (0x00-0x1F, 0x7F, covering CR/LF); baggage values SHALL additionally reject the reserved delimiters `,`, `=`, and `;` so `format_baggage`/`parse_baggage` remain round-trip-lossless. `TraceContextV1` is a distinct type from `policy::CorrelationId`; it is never derived from or interchangeable with a `CorrelationId`.
3. `SpanEventV1` binds a validated `EventName`/`EventTarget`, a `TraceContextV1`, an optional `parent_span_id: SpanId` (a sibling field, not part of `TraceContextV1`), a `start`/`end` `Timestamp` pair (`end` SHALL NOT precede `start`), a `SpanStatus` (`Unset`, `Ok`, or `Error`), and bounded `attributes` under the same grammar/ceilings as log attributes. `MAX_SPAN_BYTES` bounds the aggregate name/target/attribute size.
4. `MetricEventV1` binds a validated `EventName`, a `MetricKind` (`Counter`, `Gauge`, or `Histogram`), a finite `f64` value (`NaN`/`Infinity`/`-Infinity` SHALL be rejected as `InvalidMetric`; subnormal and full-range finite values are accepted), an optional `unit` (at most `MAX_METRIC_UNIT_BYTES`), and bounded `attributes`. `MAX_METRIC_BYTES` bounds the aggregate size; unlike spans, per-field ceilings alone can exceed `MAX_METRIC_BYTES`, so this aggregate check is load-bearing, not redundant.
5. `SpanSink` and `MetricSink` are additive ports, deliberately separate from `TelemetrySink` (one port per OTEL signal type). Both are synchronous, object-safe, `Send + Sync`, with a blanket `Arc<T>` impl, matching the existing port pattern.
6. `propagation` (framework-free, zero I/O, zero transport dependency) provides W3C Trace Context wire-format functions: `parse_traceparent`/`format_traceparent` (version `00` only; hex decoding is lowercase-only per the W3C `HEXDIGLC` grammar — uppercase SHALL be rejected, not accepted, so `format_traceparent` output round-trips byte-for-byte), `parse_trace_state`/baggage helpers, and `extract`/`inject` composing a full `TraceContextV1` from/to caller-supplied header strings. `parse_baggage` SHALL bound its raw input length upfront (mirroring `parse_trace_state`), not solely by distinct-key count, so a single-repeated-key input cannot bypass the entry-count guard.

## 7b. OpenTelemetry metric adapter

Exact-pinned `opentelemetry =0.32.0`, `default-features = false`, with `features = ["logs", "metrics"]` (adding `metrics` to the existing `logs` feature already used by §7). No `opentelemetry_sdk` or `opentelemetry-otlp` dependency: this brick never constructs an SDK, exporter, provider, or runtime, matching §7.1's existing rule.

Span export was originally out of scope for this section (issue #57) because `opentelemetry::trace::Tracer::build_with_context` has no path to make a built span carry a caller-chosen `span_id` — only the concrete `Tracer`/`IdGenerator` implementation mints span identity. That blocker does not apply to the lower-level `opentelemetry_sdk::trace::SpanExporter`/`SpanData` seam, which constructs the exported record directly rather than asking a `Tracer` to mint one; §7c specifies the adapter built against that seam ([Issue #62](https://github.com/bannff/Rust-Factory/issues/62)).

1. `OpenTelemetryMetricSink` SHALL receive an injected `opentelemetry::metrics::Meter` (a concrete type in `opentelemetry` 0.32.0, not a trait — the sink is not generic over it, unlike `OpenTelemetryLogsSink<L: Logger>`). It SHALL NOT construct an SDK, exporter, runtime, batch processor, network client, or shutdown hook, matching §7.1's existing rule. `guarantees()` SHALL report `durable_across_restart`, `visible_across_processes`, `delivery_confirmed`, and `queryable` as `false`, and `may_block` as `true` (same unverifiable-injected-dependency rationale as §7.5, per §3.3).
2. Metric egress SHALL contain only: the validated `MetricEventV1.name` mapped to the OTEL instrument name, `kind` mapped to the corresponding counter/gauge/histogram instrument type, `value`, and `unit` if present. The adapter SHALL NOT export `tenant_id` or `MetricEventV1.attributes`. There is no type-level trust distinction between `MetricEventV1.attributes` and `TelemetryEventV1.attributes` — both are `BTreeMap<String, String>` validated by the identical `validate_attribute_key` rule — so this matches §7.4's log data-minimization boundary exactly, not a relaxation of it.
3. The adapter SHALL cache one instrument per distinct `(EventName, MetricKind)` pair behind a poison-fail-closed mutex (returning `ObservabilityError::AdapterFailure` on a poisoned lock, matching §5.6's existing `LocalTelemetry` pattern) rather than constructing an instrument per `emit` call. A caller that reuses the same `EventName` with a different `MetricKind` across calls SHALL receive `ObservabilityError::InvalidMetric` from the cache lookup rather than silently reusing a mismatched cached instrument. `unit` on a subsequent call to an already-cached `(EventName, MetricKind)` pair is informational only: OTEL instrument metadata such as unit is fixed at creation, so the cached instrument's original `unit` is retained and a later call's differing `unit` value is silently not applied to the instrument. The cache is unbounded in distinct-name cardinality over the process lifetime; a caller with an unbounded or dynamically-generated metric-name pattern can grow it without limit, since no per-field ceiling in §2 bounds cache entry count.
4. Baggage is a **permanent scope exclusion** for every adapter in this brick's V1, not an open question deferred to later implementation choice: no sink (log, metric, or span) forwards `TraceContextV1.baggage` in any form. Exporting baggage requires a separately specified, explicitly opt-in mechanism with its own data-minimization contract; it SHALL NOT be bundled implicitly into any exporter's default attribute or context path.

## 7c. OpenTelemetry span adapter

Adds `opentelemetry_sdk =0.32.0` (`default-features = false`, `features = ["trace"]`) and `tokio` (`features = ["rt-multi-thread"]`) to the `opentelemetry` feature, alongside the workspace `opentelemetry` dependency's own `features` list gaining `"trace"` (needed for `SpanKind`/`Status`/`InstrumentationScope`, which the API-level `opentelemetry` crate — not `opentelemetry_sdk` — defines). Depending on `opentelemetry_sdk::trace::SpanExporter`'s trait definition is not the same as constructing an SDK: `OpenTelemetrySpanSink<E: SpanExporter>` never builds a `TracerProvider`, `BatchSpanProcessor`, or exporter's own network client; it only calls `export` on an exporter instance a composition root already built and injected, matching §7.1's existing rule exactly.

1. `OpenTelemetrySpanSink<E>` SHALL be generic over an injected `E: opentelemetry_sdk::trace::SpanExporter`, mirroring `OpenTelemetryLogsSink<L: Logger>`'s existing generic-injection pattern (not a `dyn` object — `SpanExporter` is not `dyn`-compatible).
2. `SpanSink::emit` SHALL remain synchronous per §3.1; `SpanExporter::export` is `async fn`. The adapter SHALL bridge the two internally via `tokio::task::block_in_place` wrapping `Handle::block_on`, using an explicitly injected `tokio::runtime::Handle` (never `Handle::current()` implicitly). A bare `Handle::block_on` without `block_in_place` SHALL NOT be used: it panics if `emit` is ever called from a thread that is itself a Tokio runtime worker. This bridge requires an `rt-multi-thread` runtime; the adapter is unusable under a `current_thread` runtime and SHALL NOT be constructed with one. `guarantees()` SHALL report `may_block: true`: unlike the log/metric adapters (where `may_block: true` reflects an unobservable injected dependency), this adapter itself performs the blocking.
3. The adapter SHALL export exactly one span per `emit` call (`export(vec![span_data])`) with no internal buffering or batching; a `BatchSpanProcessor` remains something this brick SHALL NOT construct.
4. `trace_id`, `span_id`, and `trace_flags` SHALL be direct byte-for-byte conversions from this crate's own validated, non-zero-enforced `TraceId`/`SpanId`/`TraceFlags` newtypes to their `opentelemetry` equivalents, with no re-validation and no minting — this is the seam #57 could not use (`Tracer::build_with_context` mints its own identity and cannot accept a caller-chosen `span_id`); `SpanContext::new` accepts caller-supplied identity directly, preserving it exactly. `parent_span_id` SHALL map `None` to `opentelemetry::trace::SpanId::INVALID` (OTEL's own convention for "no parent," not an invented value).
5. `trace_state` SHALL be forwarded on a best-effort basis: this crate validates only its byte length and control-byte safety (§7a.2), not the W3C `tracestate` list-member grammar `opentelemetry::trace::TraceState`'s parser enforces, so a value that fails to parse there SHALL be forwarded as an empty `TraceState` rather than failing the whole `emit`. This is acceptable because `trace_state` is optional presentational metadata, not an identity-bearing field like `trace_id`/`span_id`.
6. Egress SHALL exclude `tenant_id` and `SpanEventV1.attributes`, matching §7b.2's data-minimization boundary exactly (no type-level trust distinction between attribute maps). `span_kind` SHALL be the fixed value `SpanKind::Internal` and `parent_span_is_remote` SHALL be the fixed value `false`: neither has a source field on `SpanEventV1`/`TraceContextV1`, and asserting a stronger value with no evidence would be the same class of unverified optimistic claim §3.3 forbids for `may_block`. `dropped_attributes_count` SHALL be `0` (`validate_span` already ran before `emit` constructs the record). `events` and `links` SHALL be empty (`SpanEventV1` has no sub-event or link concept). `instrumentation_scope` SHALL use this crate's existing `rust_factory.observability` target constant and the compiling crate's own `CARGO_PKG_VERSION`.

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
5. Status output SHALL contain only the five `TelemetryGuarantees` booleans and SHALL not call the reader.
6. Public failures SHALL project only the stable snake-case codes. Source failure, Policy denial, digest tampering, reader failure, and unexpected serialization detail SHALL not leak internal text.
7. The MCP surface is read-only. It SHALL NOT expose event emission, configuration mutation, exporter control, or lifecycle operations.

## 9. Test strategy and checked evidence

- `tests/public_contract.rs` — framework-free identifier/label grammar, byte/count limits, half-open query semantics, stable non-leaking errors, tenant/clock stamping, hostile-reader filtering, deployment ceilings, guarantees composition, and object-safe trait use.
- `tests/local_contract.rs` plus the local module test — adapter ingress/query validation, tenant isolation, deterministic sequence/order/eviction, conjunctive filtering, concurrent writers, global cardinality, and poisoned-state failure.
- `tests/settings_contract.rs` — closed Serde/Schemars shapes, semantic limits, stable backend names, and both exact TOML forms.
- `tests/opentelemetry_contract.rs` — all severity mappings, metadata-only egress for logs, metrics, and spans alike, reserved-attribute and baggage exclusion, disabled events, instrument-kind-mismatch rejection, and submit-only guarantees. Its `span_sink` module covers byte-exact trace/span/parent identity preservation, tenant/attribute/baggage exclusion, every `SpanStatus` mapping, exporter-failure projection to `AdapterFailure`, and the `block_in_place` sync/async bridge under both a spawned-blocking-task call and the actual adversarial case (`emit` called directly within an async runtime worker task, which panics without the bridge and is asserted not to).
- `src/propagation.rs`'s own `#[cfg(test)]` module — W3C `traceparent` parse/format round-trip, lowercase-hex-only rejection, all-zero trace/span id rejection, `trace_state`/baggage control-byte and reserved-delimiter rejection, bounded raw-input length, and `extract`/`inject` composition.
- `tests/public_contract.rs` (trace/span/metric additions) — `TraceId`/`SpanId`/`TraceFlags` construction and boundaries, `TraceContextV1`/`SpanEventV1`/`MetricEventV1` validation and aggregate-ceiling behavior, `SpanSink`/`MetricSink` object-safety, and the new error taxonomy's stable non-leaking codes.
- `src/mcp.rs` contract tests — exact tool/schema surface, byte-exact request ceiling before authorization, semantic validation only after authorization, Policy deny/tamper/source failures before reader effects, trusted tenant derivation, exact capabilities, raw and escaped/framed safe record/status projection, and error redaction.
- `policy` tests bind both Observability capability wire names into Policy decision digests.

The narrow delivery command is `cargo test -p observability --all-features`. The final workspace acceptance command remains `make check` and is not evidence until it completes successfully.

## 10. Named gaps

None of these is claimed as done:

1. **No Agent or Workflow integration.** Neither emits through this brick; no autonomous runtime path demonstrates operational telemetry end to end.
2. **No durable reader or exporter.** Local records disappear with the process and evict at bounded capacity. The OpenTelemetry log/metric adapters accept only an API logger/meter and prove neither exporter construction nor exporter delivery. The span adapter (§7c) exports to an injected `SpanExporter` but constructs no SDK, provider, or batch processor itself, matching the same rule.
3. **No audit trail.** Logs, spans, and metrics are operational telemetry and cannot replace Workflow/Evaluation evidence. **No baggage export.** No adapter in this brick (log, metric, or span) forwards `TraceContextV1.baggage`; see §7b.4.
4. **No composition binary.** Settings source, adapter construction, trusted context, Policy composition, OpenTelemetry SDK/exporter/runtime, transport binding, and orderly flush/shutdown are unimplemented.
5. **No `cargo-audit` gate.** `make check` has no advisory scan, so the exact OpenTelemetry pin has no repository-enforced advisory monitoring.
6. **No durable retention or recovery contract.** Retention periods, retries, acknowledgement, replay, crash recovery, and cross-process querying require separately specified adapters and lifecycle ownership.
