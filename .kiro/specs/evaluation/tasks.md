# Tasks: Evaluation

- [x] 1. Create `evaluation` with versioned definitions, criteria, evidence refs, verdicts, immutable result records, digests, and errors.
- [x] 2. Define read-only `WorkflowEvidenceReader` and create-or-match `EvaluationStore` ports.
- [x] 3. Implement deterministic exact-output and closed predicate criteria with fail-closed evidence validation.
- [x] 4. Create `evaluation-memory` deterministic workflow-evidence reader and immutable result store.
- [x] 5. Test tenant isolation, non-terminal/malformed evidence rejection, PASS/FAIL/ERROR distinction, content-hash stability, and create-or-match conflicts.
- [x] 6. Create `evaluation-mcp` with trusted-context injection and validate/evaluate_run/get_result operations.
- [x] 7. Contract-test MCP schemas, safe public errors, immutable result projections, and absence of Workflow mutation capability.
- [x] 8. Accept the initial Evaluation vertical slice: QA/security/architecture approvals and `make check` passed.
- [x] 9. Accept the separate Evaluation MCP Policy compatibility migration: verified host context/Policy authorization, exact capability mapping, bounded stdio ingress, QA/security/Rust SME/architecture approval, and `make check` recorded in `../evaluation-policy-compatibility/`.

## Rust design-gate constraints

- [x] Define `TerminalEvidenceSnapshotV1`, canonical-byte encoding, semantic SHA-256 content hash, tenant-scoped reader, and exact logical evaluation key before implementation.
- [x] Define closed criterion variants and all byte/count/event/finding ceilings before implementation.
- [x] Add contract tests for canonical/hash stability and mutation, tenant non-disclosure, snapshot integrity, concurrent create-or-match, criterion ordering/limits, and malformed evidence ERROR behavior.
- [x] Keep `evaluation` independent of `workflow-memory`, MCP, and concrete `workflow::Run`; `evaluation-memory -> evaluation + workflow`; `evaluation-mcp -> evaluation + MCP` with injected ports.

- [x] Add golden byte/hash vectors for V1 snapshot, definition, and immutable result canonical encodings.
- [x] Implement tenant-first `EvaluationStore::get`/`list` and test cross-tenant NotFound behavior.

## Validation matrix and evidence

Narrow command: `cargo test -p evaluation -p evaluation-memory -p evaluation-mcp`. Required: canonical-byte/hash golden vectors, criteria PASS/FAIL/ERROR, tenant non-disclosure, create-or-match conflict/concurrency, MCP schema/projection. Conditional: fuzz canonical codecs; loom for future store synchronization changes. N/A: generated fixture and sandbox tests.

Implementation/QA evidence: evaluation, evaluation-memory, and evaluation-mcp are present; QA added golden vectors, semantic mutations, closed criteria, malformed/cross-tenant ERROR behavior, immutable store concurrency, and MCP contract coverage. Initial-slice security and architecture approval is recorded above; Policy compatibility is deliberately a separate migration.