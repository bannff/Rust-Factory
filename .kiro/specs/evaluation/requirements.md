# Requirements: Evaluation

Evaluation independently assesses immutable terminal Workflow evidence. It SHALL not invoke Agents, mutate Workflow runs, retry execution, choose lifecycle transitions, or own process/runtime lifecycle.

## Completed V1 capability

1. `evaluation` SHALL own versioned criteria, evidence snapshots and references, verdicts, findings, immutable result records, canonical encodings, digests, and typed errors.
2. Evaluation SHALL consume a terminal snapshot through the injected, tenant-scoped, read-only `WorkflowEvidenceReader` port. The brick SHALL NOT depend on `workflow` or provide a Workflow evidence adapter. Issue [#16] owns the production Workflow-to-Evaluation bridge and runnable composition proof.
3. `EvaluationExecutor` SHALL remain an object-safe, framework-neutral port. It receives validated core definitions and evidence and returns only a core-owned verdict with ordered findings; framework types SHALL NOT enter core signatures.
4. The `local` feature SHALL expose `local::DeterministicCriteriaEvaluator`, the runtime-free reference implementation for exact output and the two closed event predicates.
5. The `serdes-ai-evals` Cargo feature SHALL expose `serdes_ai_evals::SerdesAiEvalsExecutor`. The adapter SHALL contain `serdes-ai-evals`, preserve V1 byte-comparison semantics and ordered findings, and SHALL NOT claim model judging, network access, or runtime ownership.
6. Executor futures SHALL use the standard-library `Future` seam. Callers own polling and cancellation. Dropping a future cancels only by drop; executors SHALL start no detached work and claim no cancellation acknowledgement, cross-process cancellation, timeout, retry, or recovery guarantee.
7. Core service code SHALL retain ownership of tenant/run binding, evidence and criterion digests, logical result identity, canonical result bytes, content hashes, and assessment validation. An executor cannot supply or override those fields.
8. `EvaluationStore` SHALL create-or-match immutable content-addressed records; conflicting identity/content is an error. Reads SHALL be tenant-first, and cross-tenant access SHALL be indistinguishable from absence.
9. The `memory` feature SHALL expose only bounded process-local result storage through `memory::InMemoryEvaluationStore`. It SHALL NOT adapt Workflow evidence and SHALL claim no persistence, restart durability, cross-process visibility, or crash atomicity. Default hard ceilings are 1,024 results per tenant and 4,096 globally; reaching either ceiling refuses growth without eviction.
10. The `settings` feature SHALL expose closed Serde/Schemars V1 configuration for `local_deterministic`, `serdes_ai_evals`, and bounded `in_memory` storage. A composition root owns configuration sources, feature-availability errors, and backend construction.
11. The `mcp` feature SHALL preserve exactly three tools: `evaluation_validate`, `evaluation_evaluate_run`, and `evaluation_get_result`. Their exact Policy capabilities remain `EvaluationValidate`, `EvaluationEvaluate`, and `EvaluationGet`, respectively.
12. MCP handlers SHALL first enforce only the serialized parameter-DTO size ceiling, then derive trusted context and authorize the exact capability, then perform semantic validation and any reader/store effect. Denied requests SHALL NOT reveal semantic validity and SHALL make no reader/store call.
13. Evaluation's MCP adapter SHALL NOT claim a full MCP or JSON-RPC envelope limit. Bounding before envelope buffering/deserialization, stdio or other transport binding, Tokio startup, and orderly shutdown belong to the composition transport.
14. Allowed MCP operations SHALL preserve tenant-first lookup, immutable create-or-match behavior, safe public projections, and unchanged V1 semantics. Evaluation results may be linked by later Workflow projections but SHALL never alter Workflow state.

Non-goals: model-judged evaluation, UI/dashboard suites, experiment runners, arbitrary public scorer callbacks, direct provider configuration, Workflow control, durable persistence, transport lifecycle, or mesh replication.

## Normative evidence and persistence contract

1. `TerminalEvidenceSnapshotV1` is an Evaluation-owned projection. Core rejects a tenant/run mismatch, non-terminal status/reason pair, missing or malformed identity/scope fields, malformed event ordering, or an exceeded bound. Invalid evidence produces an ERROR result when a snapshot was returned; absence remains not-found.
2. Canonical result bytes encode schema version, tenant ID, evaluator ID/version, logical evaluation key, criterion digest, terminal evidence digest, verdict, and ordered findings with length-prefixed UTF-8 fields. SHA-256 hashes those bytes; backend selection and request-specific Policy decisions are not semantic result content.
3. `EvaluationStore::create_or_match` atomically returns Created only when absent, Existing only when canonical content matches, and Conflict when the same logical key resolves to different content. The logical key is `(tenant, evaluator_id, evaluator_version, criterion_digest, workflow_run_id, workflow_revision)`.
4. Criteria are closed and bounded: `ExactOutput`, `EventKindCount { kind, expected }`, and `EventDataEquals { sequence, expected }`. Violated criteria yield FAIL; invalid evidence or an invalid executor assessment yields ERROR/adapter failure rather than a false FAIL.
5. `EvaluationStore` exposes tenant-scoped `get` and `list` in addition to create-or-match. Cross-tenant reads return absence.

## V1 wire schema, limits, and compatibility vectors

`TerminalEvidenceSnapshotV1` fields, in canonical order: `schema_version="v1"`; `tenant_id`; `run_id`; `workflow_id`; `workflow_version`; `run_revision` as unsigned decimal; `terminal_status` (`succeeded|failed|cancelled`); `terminal_reason`; `attempt_id`; `agent_id`; `capability_scope_digest` (64 lowercase hexadecimal SHA-256); `output`; and ordered `events[] { sequence, kind, data }`.

Canonical bytes use UTF-8 fields in the stated order. Each string is encoded as `<byte_length>:<bytes>\n`; unsigned integers use ASCII decimal encoded by the same field rule; enums use their exact lowercase spelling; lists encode their count followed by each item. No Unicode normalization, trimming, or implicit ordering occurs beyond explicit event sequence.

Events begin at sequence 1 and increase by one. The limits are 16 criteria, 64 events, 16 KiB expected/output strings, 4 KiB per event kind+data pair, 64 KiB aggregate canonical snapshot bytes, 32 findings, and 4 KiB per finding. `terminal_reason` is closed: `succeeded/completed`, `failed/invocation_failed`, and `cancelled/cancelled` are the only valid pairs.

The V1 golden cohort is exact and unchanged:

- snapshot digest: `400d023425c9ee77e3eb9ac40032e0871dcc3eaf6980b743f29fccdc025150eb`
- definition/criterion digest: `5c94014a3ba627135274d1cf4c9b54e2c06af1a24e396d8d6dc3c5f6ab90d401`
- result content hash: `03414bc05e2c0b4aae494cc0fe12473da48fa0922f637e3836662839a5bebe72`

Both V1 executors SHALL reproduce the same verdict, ordered findings, digests, and result hash for the shared cohort.

[#16]: https://github.com/bannff/Rust-Factory/issues/16
