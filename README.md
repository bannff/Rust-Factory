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
| `crates/policy` | `memory` | Trusted context, closed capabilities, and grant decisions. No MCP surface, by design. |
| `crates/memory` | `local`, `agentic`, `settings` | Tenant-scoped agent memory behind one framework-agnostic port. Two selectable backends; no durable adapter and no MCP surface yet. |
| `crates/{model-gateway,sandbox,observability}` | — | Status-only scaffolds for the families the autonomous loop drives next; their provisional ports still live in `agent`. |
| `crates/mcp-transport` | — | Shared bounded MCP stdio transport. Owns no capability. |

A `memory`, `local`, or `fs` module is a deterministic process-local adapter: no
persistence, recovery, lease, or cross-process guarantee. A vendor module is named
for the crate it confines — `agentic` holds `agentic-memory` and nothing else
names it — and a `settings` module holds the shape of a project's configuration,
never its source and never the selection-to-constructor `match`, which belong to a
composition binary. An adapter is feature-gated even when it adds no dependency,
so the rule that a core module names no adapter keeps applying to it.

`policy` has no MCP surface deliberately. It decides what an agent is permitted
to do, so exposing it to agents would be a privilege-escalation seam whichever
tools were chosen. Every other operable brick is agent-drivable.

Every package declares `family`, `role`, and `status` in `[package.metadata.rust-factory]`, and the [Vision portfolio registry](.kiro/steering/living-factory-vision.md#brick-portfolio-registry) is the family-level source of truth. Capabilities that are committed but not yet driven by a consumer — workspace governance, identity, knowledge, verification, message bus, cache, graph, notification — are registry rows naming a future crate rather than empty packages, so a new concern always has a designated home without shipping code that does nothing. `make check` enforces registry and metadata agreement in both directions.

## Repository layout

- `crates/` — libraries, one per capability. No binary targets.
- `projects/` — deployable binaries, one per composition root. Not yet created.
- `.kiro/steering` — architecture rules, injected into every agent session.
- `.kiro/specs/brick-standard` — the contract a new or refactored brick follows.
- `.kiro/skills` — shared Rust guidance for all agent roles.

## Build

Requires Rust 1.88+ (edition 2024) and Python 3.11+, which the registry
validator needs for `tomllib`.

```sh
cargo build --workspace   # framework-free cores only
make check                # the full gate, across the feature matrix
```

The workspace produces libraries only. There is nothing to run yet: transport
binding belongs to a `projects/` composition root, and none exists ([#6]).

[#6]: https://github.com/bannff/Rust-Factory/issues/6

## Brick standard

New or refactored bricks follow the [Canonical Brick Standard](.kiro/specs/brick-standard/requirements.md). A brick is exactly one crate, named for its capability. Adapters are feature-gated modules inside it — `mcp`, `memory`, `local`, `fs`, a vendor module, `settings` — and a binary under `projects/` owns runtime, transport, configuration, trusted context, Policy composition, concrete adapter injection, and shutdown. Boundary DTOs use `serde` and `schemars`; typed constructors and core `validate_*` rules establish domain validity. No library owns process lifecycle: `serve_stdio` has been removed from every brick.

A shared contract is extracted only after a demonstrated stable need has at
least two consumers; the extracted crate becomes canonical and consumers depend
inward on it. There is no generic umbrella core.

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
