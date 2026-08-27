# Rust Factory

Rust Factory is a domain-agnostic foundation for reliable Rust services, libraries, and tools.

## Workspace

- `crates/factory-core` — shared Factory primitives, intentionally minimal until a second brick needs them.
- `crates/project-core` — deterministic project-blueprint validation and generation planning.
- `crates/project-fs` — root-confined project materialization.
- `crates/project-mcp` — bounded MCP project tools.
- `crates/policy-core` — transport-independent trusted context, closed capability, and grant decision contracts.
- `crates/policy-memory` — deterministic process-local static policy grant resolver; no persistence or cross-process guarantees.
- `crates/agent-core` — versioned agent definitions, policy, registry, and local runtime contracts.
- `crates/agent-mcp` — bounded MCP agent controls.
- `crates/workflow-core` — transport-independent bounded workflow lifecycle contracts.
- `crates/workflow-memory` — deterministic process-local, in-memory workflow adapters; no persistence, recovery, leases, or cross-process cancellation guarantees.
- `crates/workflow-mcp` — bounded MCP workflow controls.
- `crates/evaluation-core` — deterministic immutable evaluation contracts for terminal workflow evidence.
- `crates/evaluation-memory` — deterministic process-local, in-memory evidence reader and result store; no persistence or cross-process durability guarantees.
- `crates/evaluation-mcp` — bounded MCP evaluation controls.
- `.kiro/skills` — shared Rust guidance for all agent roles.
- `.kiro/specs/rust-factory-foundation` — the first capability-oriented migration plan.

## Quality gate

Run `make check` before submitting changes. It formats, lints, and tests the entire workspace.
