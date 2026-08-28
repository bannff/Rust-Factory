# Tasks: Workflow

- [x] 1. Create `workflow` with typed definitions, context, run/attempt/event/status/terminal-reason/budget/error contracts.
- [x] 2. Define core-owned `WorkflowStore` and `AgentInvoker` ports plus canonical idempotency identity.
- [x] 3. Implement validation, tenant-safe visibility, legal state transitions, and bounded evidence rules.
- [x] 4. Create `workflow::memory` with deterministic in-memory store and static agent invoker adapters.
- [x] 5. Implement idempotent start, single attempt execution, append-only evidence, terminal success/failure, get/list, and cancel.
- [x] 6. Add focused tests for duplicate key replay, key conflict, tenant isolation, cancellation-versus-late-completion, terminal immutability, and evidence persistence.
- [x] 7. Create `workflow::mcp` with injected store/invoker ports and bounded validate/start/get/list/cancel operations.
- [x] 8. Add MCP schema/error/tenant-safety contract tests and safe response projections.
- [x] 9. Close workflow review blockers: typed exact start keys, atomic failed terminalization on evidence limits, store mutation invariants, authorized validation, bounded evidence streaming, and honest local-only cancellation semantics.
- [x] 10. Run Factory quality gates and conduct security/architecture review before marking Workflow stable.
- [x] 11. Historical note: Evaluation is specified and implemented as a separate immutable evidence-assessment brick; final security and architecture gates remain pending.

## Architecture constraints

- `workflow` may depend only on stable core contracts, including `agent` for `AgentId`.
- `workflow::memory` depends on `workflow`; `workflow::mcp` depends on `workflow` plus MCP/serialization crates. Neither is a `workflow` dependency.
- Add deterministic tests for atomic transition conflicts and cancellation-versus-late-completion publication.
- Derive tenant/principal context from an injected authenticated-session resolver, never MCP request fields.

## Validation matrix

Narrow command: `cargo test -p workflow --features mcp,memory`. Required: idempotency, tenant isolation, transitions, bounded evidence, local cancellation race, MCP context. Conditional: proptest start identities; loom and recovery tests only with persistent/concurrent adapters; fuzz canonical JSON. Golden vectors apply to canonical input identities only.