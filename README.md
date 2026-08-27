# Rust Factory

Rust Factory is a domain-agnostic foundation for reliable Rust services, libraries, and tools.

## Workspace

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
- `crates/{model-gateway,memory,sandbox,observability}-core` — zero-dependency status-only scaffolds for the four capability families the autonomous loop drives next; their provisional ports still live in `agent-core`.

Every package declares `family`, `role`, and `status` in `[package.metadata.rust-factory]`, and the [Vision portfolio registry](.kiro/steering/living-factory-vision.md#brick-portfolio-registry) is the family-level source of truth. Capabilities that are committed but not yet driven by a consumer — workspace governance, identity, knowledge, verification, message bus, cache, graph, notification — are registry rows naming a future crate rather than empty packages, so a new concern always has a designated home without shipping code that does nothing. `make check` enforces registry and metadata agreement in both directions.
- `.kiro/skills` — shared Rust guidance for all agent roles.
- `.kiro/specs/rust-factory-foundation` — the first capability-oriented migration plan.

Shared capability contracts are introduced only as narrowly named, transport-independent cores after a demonstrated stable contract has at least two consumers. The extracted core becomes canonical; consumers depend inward on it. Rust Factory intentionally has no generic umbrella core.

## Brick standard

New or refactored bricks follow the Rust-SME-approved [Canonical Brick Standard](.kiro/specs/brick-standard/requirements.md). A capability owns a small framework-free `-core`; deterministic local state belongs in an optional `-memory` adapter; MCP is an optional bounded `-mcp` adapter library; and an optional `-server` binary owns runtime, transport, configuration, trusted context, Policy composition, concrete adapters, and shutdown. Boundary DTOs use `serde` and `schemars`; typed constructors and core `validate_*` rules establish domain validity. The standard also tracks the behavior-preserving migration that removes stdio lifecycle ownership from existing MCP libraries.

## Quality gate

Run `make check` before submitting changes. It formats, lints, and tests the entire workspace.
