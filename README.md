# Rust Factory

Rust Factory is a domain-agnostic foundation for reliable Rust services, libraries, and tools.

## Workspace

One brick is one crate. Its agent-facing MCP surface and its local adapters are
feature-gated modules inside it, so a capability is one thing to find and compose.
No feature is on by default, so a brick's default build resolves no transport,
schema, error-framework, filesystem, or async-runtime dependency.

| Brick | Features | Capability |
|---|---|---|
| `crates/agent` | `mcp` | Versioned agent definitions, registry, and bounded local runtime contracts. |
| `crates/workflow` | `mcp`, `memory` | Bounded workflow lifecycle for one agent invocation. |
| `crates/evaluation` | `mcp`, `memory` | Immutable evaluation contracts over terminal workflow evidence. |
| `crates/project` | `mcp`, `fs` | Blueprint validation, generation planning, and root-confined materialization. |
| `crates/policy` | `memory` | Trusted context, closed capabilities, and grant decisions. No MCP surface by design — see below. |
| `crates/{model-gateway,memory,sandbox,observability}` | — | Status-only scaffolds for the families the autonomous loop drives next; their provisional ports still live in `agent`. |
| `crates/mcp-transport` | — | Shared bounded MCP stdio transport. Owns no capability. |

A `memory` or `fs` module is a deterministic process-local adapter: no
persistence, recovery, lease, or cross-process guarantee.

`policy` has no MCP surface deliberately. It decides what an agent is permitted
to do, so exposing it to agents would be a privilege-escalation seam whichever
tools were chosen. Every other operable brick is agent-drivable.

Every package declares `family`, `role`, and `status` in `[package.metadata.rust-factory]`, and the [Vision portfolio registry](.kiro/steering/living-factory-vision.md#brick-portfolio-registry) is the family-level source of truth. Capabilities that are committed but not yet driven by a consumer — workspace governance, identity, knowledge, verification, message bus, cache, graph, notification — are registry rows naming a future crate rather than empty packages, so a new concern always has a designated home without shipping code that does nothing. `make check` enforces registry and metadata agreement in both directions.
- `.kiro/skills` — shared Rust guidance for all agent roles.
- `.kiro/specs/rust-factory-foundation` — the first capability-oriented migration plan.

Shared capability contracts are introduced only as narrowly named, transport-independent cores after a demonstrated stable contract has at least two consumers. The extracted core becomes canonical; consumers depend inward on it. Rust Factory intentionally has no generic umbrella core.

## Brick standard

New or refactored bricks follow the Rust-SME-approved [Canonical Brick Standard](.kiro/specs/brick-standard/requirements.md). A brick is exactly one crate, named for its capability. Adapters are feature-gated modules inside it — `mcp`, `memory`, `fs` — and a binary under `projects/` owns runtime, transport, configuration, trusted context, Policy composition, concrete adapter injection, and shutdown. Boundary DTOs use `serde` and `schemars`; typed constructors and core `validate_*` rules establish domain validity. The standard also tracks the behavior-preserving migration that removes stdio lifecycle ownership from existing MCP libraries.

## Quality gate

Run `make check` before submitting changes. It validates the brick registry,
asserts adapter isolation, formats, lints, and tests — each across the feature
matrix, because with no default features a workspace-wide command would only
exercise the framework-free cores.

`make isolation-check` asserts that each brick's default build resolves none of
the adapter dependencies; the registry validator separately forbids naming one
outside its own module. Neither claims framework-free *artifacts*: Cargo unifies
features per build graph, so a binary composing several bricks with `mcp` enabled
links one framework-carrying build of each.
