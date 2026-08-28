# Design: Evaluation

```text
workflow evidence (read-only)
          ↓
evaluation
  EvaluationDefinition / EvidenceRef / Result / Verdict
  WorkflowEvidenceReader + EvaluationStore
          ↑
evaluation::memory  deterministic reader/store adapters
          ↑
evaluation::mcp     validate / evaluate_run / get_result
```

`evaluation` defines a deterministic criterion and canonical immutable record. `WorkflowEvidenceReader` returns only a tenant-authorized terminal snapshot. `EvaluationStore` persists by content hash using create-or-match semantics. The MCP adapter injects both ports and never receives a Workflow mutator or Agent invoker.

The initial `ExactOutputCriterion` compares a bounded expected string with terminal normalized output. `PredicateCriterion` may inspect typed event kinds and values through a closed predicate enum. Evaluator errors are distinct from a FAIL verdict. Canonical records use stable field ordering and SHA-256 content hash over semantic fields; timestamps are metadata, not part of the hash.

Next implementation sequence: core contracts/validation → in-memory evidence reader/store → immutable evaluation → MCP controls → evidence/provenance/tenant contract tests → review. Workflow remains a producer of evidence; Evaluation remains a consumer.

## Rust SME refinements

`evaluation` does not consume `workflow::Run` directly. It owns a versioned `TerminalEvidenceSnapshotV1` projection and reads it through a tenant-scoped port. Canonical bytes are built only in core using length-prefixed UTF-8 fields; tenant scope is semantic and hashed. `evaluation::memory` may adapt `workflow` into that projection, while `evaluation::mcp` depends only on `evaluation` plus MCP/serialization and receives reader/store/trusted-context ports by injection.

The core supplies complete immutable records to an atomic create-or-match store. A logical key conflict is distinct from content equality. Contract tests cover canonical/hash equivalence and mutation, tenant non-disclosure, terminal snapshot integrity, concurrent create-or-match, each closed criterion, criterion limits, and malformed evidence ERROR behavior.

## Normative V1 representation

The V1 snapshot/result codec is owned solely by `evaluation`: length-prefixed UTF-8 fields (`<bytes>:<field>\n`), fixed field ordering, ASCII decimal integers, exact lowercase enums, and explicit list counts. The precise `TerminalEvidenceSnapshotV1`, `EvaluationDefinitionV1`, closed `CriterionV1`, limits, digest, and tenant-first lookup rules are normative in `requirements.md`. `EventDataEquals` is raw UTF-8 byte equality; no extensible predicate/callback API exists.
