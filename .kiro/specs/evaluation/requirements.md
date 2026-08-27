# Requirements: Evaluation

Evaluation independently assesses immutable terminal Workflow evidence. It SHALL not invoke Agents, mutate Workflow runs, retry execution, or choose lifecycle transitions.

1. `evaluation` SHALL own versioned criteria, evidence references, verdicts, findings, immutable result records, and typed errors.
2. Evaluation SHALL resolve a terminal Workflow evidence snapshot through an injected read-only `WorkflowEvidenceReader` port and fail closed when evidence is missing, malformed, non-terminal, or cross-tenant.
3. The first evaluator SHALL be deterministic: exact expected output and bounded predicates over normalized terminal evidence.
4. Every result SHALL include evaluator ID/version, criterion/input digest, evidence reference, verdict (`pass`, `fail`, or `error`), findings, and immutable content hash.
5. `EvaluationStore` SHALL create-or-match immutable content-addressed records; conflicting identity/content is an error.
6. `evaluation::mcp` SHALL expose `evaluation_validate`, `evaluation_evaluate_run`, and `evaluation_get_result`; it derives trusted tenant/principal context at ingress and returns safe public projections only.
7. Evaluation results may be linked by later Workflow projections but SHALL never change Workflow terminal status.

Non-goals: model-judged evaluation, UI/dashboard suites, arbitrary scorer callbacks, experiment runners, direct provider configuration, workflow control, or mesh replication.

## Normative evidence and persistence contract

1. `evaluation` SHALL own `TerminalEvidenceSnapshotV1`, copied through a tenant-scoped read-only `WorkflowEvidenceReader::get_terminal(tenant_id, run_id)` port. Core SHALL reject a tenant mismatch, non-terminal status, missing attempt/scope digest, malformed event ordering, or evidence revision mismatch as an evaluation error—not a FAIL verdict.
2. Canonical result bytes SHALL encode schema version, tenant ID, evaluator ID/version, logical evaluation key, criterion digest, full terminal evidence identity/revision/digest, verdict, and ordered findings with length-prefixed UTF-8 fields. SHA-256 hashes those bytes; timestamps are non-semantic metadata.
3. `EvaluationStore::create_or_match` SHALL atomically return Created only when absent, Existing only when canonical bytes match, and Conflict when the same logical key resolves to different content. Logical key is `(tenant, evaluator_id, evaluator_version, criterion_digest, workflow_run_id, workflow_revision)`.
4. Criteria are closed and bounded: `ExactOutput`, `EventKindCount { kind, expected }`, and `EventDataEquals { sequence, expected }`. Validation enforces criterion count, expected-string, event, finding, and snapshot byte ceilings. Unsupported or malformed data is an ERROR verdict.
5. All evaluation reads and result lookups are tenant scoped and cross-tenant access is not-found.

## V1 wire schema and limits

`TerminalEvidenceSnapshotV1` fields, in canonical order: `schema_version="v1"`; `tenant_id`; `run_id`; `workflow_id`; `workflow_version`; `run_revision` as unsigned decimal; `terminal_status` (`succeeded|failed|cancelled`); `terminal_reason`; `attempt_id`; `agent_id`; `capability_scope_digest` (64 lowercase hex SHA-256); `output`; and ordered `events[] { sequence: unsigned decimal, kind, data }`. Snapshot digest is SHA-256 of its canonical bytes. Events begin at sequence 1, increase by one, have nonempty kind, maximum 64 events, 4 KiB per kind+data chunk, and 64 KiB aggregate snapshot bytes.

Canonical bytes use UTF-8 fields in the stated order. Each string is encoded as `<byte_length>:<bytes>\n`; unsigned integers use ASCII decimal encoded by the same field rule; enums use their exact lowercase spelling; lists are encoded as count then each item. No Unicode normalization, trimming, or implicit ordering occurs beyond the explicit event sequence.

`EvaluationDefinitionV1` has `schema_version="v1"`, evaluator logical ID/version, and `criteria[]` (maximum 16). `CriterionV1` is exactly `ExactOutput { expected }`, `EventKindCount { kind, expected: u32 }`, or `EventDataEquals { sequence: u64, expected }`; comparisons are byte-for-byte UTF-8 equality. Expected/output strings are at most 16 KiB; findings are at most 32 entries and 4 KiB each. Violated criteria yield FAIL; invalid definition/snapshot/limits yield ERROR.

`EvaluationStore` SHALL expose `get(tenant_id, logical_key)` and `list(tenant_id)` in addition to create-or-match. Cross-tenant reads return NotFound. Result content hash is SHA-256 of canonical result bytes; criterion digest is SHA-256 of canonical definition bytes. Golden vectors for snapshot, definition, and result bytes/hashes are mandatory contract tests.

`terminal_reason` is required and closed: `succeeded` requires `completed`; `failed` requires `invocation_failed`; `cancelled` requires `cancelled`. Any other status/reason pair is malformed evidence and produces an ERROR verdict.
