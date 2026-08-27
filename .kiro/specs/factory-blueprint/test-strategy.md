# Test Strategy Matrix

| Brick | Required coverage | Conditional coverage |
|---|---|---|
| Project | validation, deterministic plan, filesystem confinement, MCP schema, generated `make check` fixture | property tests for path/name invariants; fuzz parsers |
| Agent | definition/policy validation, reference catalog, capability limits, tool denial, MCP schema | property scope-digest tests; loom only after shared-state adapter |
| Workflow | idempotency, tenant isolation, transitions, evidence bounds, cancellation race, MCP context | property start identities; loom/persistent-adapter recovery tests |
| Evaluation | criteria, terminal evidence validation, create-or-match, tenant non-disclosure, MCP projection, canonical-byte/hash golden vectors, concurrency store tests | fuzz canonical codecs; loom for future store synchronization changes |
| Future capability extraction | owner/consumer migration and compatibility tests | property/fuzz/loom only when its contract warrants them |
| Adapter portfolio / experiments | descriptor selection compatibility, exact Cargo plan, framework isolation, candidate/evidence provenance, no auto-promotion | framework-specific integration/load/benchmark tests; telemetry regression checks when a measured adapter exists |

Python mapping: Pydantic ingress/egress → typed Rust models plus serde/schemars at adapters; Hypothesis → proptest where invariant input space is meaningful; chaos/concurrency → deterministic race tests and loom only for synchronization; pytest fixtures → Rust builders/fixtures; immutable records → golden canonical bytes and SHA-256 vectors.

Each brick spec names its required narrow Cargo command and marks all other techniques as N/A with rationale. Local/in-memory adapters never imply production durability, recovery, or cross-process behavior.