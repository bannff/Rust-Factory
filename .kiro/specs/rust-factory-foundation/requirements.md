# Requirements: Rust Factory Foundation

## Purpose

Establish a small, idiomatic Rust foundation that proves the Factory's central promise: an agent can use MCP to turn a declarative blueprint into a clean, validated Rust workspace. Preserve Python Factory's useful architectural boundaries without reproducing its Python-specific implementation.

## Functional requirements

1. The workspace SHALL use narrowly named, framework- and transport-independent capability cores. A new shared core is created only for a demonstrated stable contract with at least two consumers; `project` owns the first brick's stable domain contracts.
2. The first vertical slice SHALL be a project brick that validates a declarative Rust project blueprint, produces a deterministic scaffold plan, and generates the planned workspace through a filesystem adapter.
3. `project` SHALL expose typed contracts for blueprints, validation findings, generated files, generation plans, project targets, materialization, and errors. It SHALL contain no MCP, filesystem, or framework types.
4. The generation plan SHALL be deterministic for the same valid blueprint and Factory version.
5. The filesystem adapter SHALL hold an opened, capability-confined target-root directory; it SHALL atomically reserve a one-component target, reject traversal, and refuse overwrite by default.
6. The project brick SHALL expose bounded MCP operations to validate a blueprint, inspect a dry-run plan, generate a workspace, and report validation results without exposing host filesystem paths or raw I/O errors.
7. Generated workspaces SHALL include the Factory's baseline Cargo layout and quality commands.
8. Blueprints and project kinds SHALL be data-driven. The authoring core SHALL not contain domain-specific orchestration branches.

## Quality requirements

1. The workspace SHALL forbid unsafe code and deny Rust 2018 idiom violations.
2. Formatting, Clippy with warnings denied, and workspace tests SHALL be the quality gate.
3. Core blueprint validation and generation planning SHALL have focused deterministic tests before filesystem or MCP adapters are added.
4. Filesystem and MCP adapters SHALL have integration tests for their public contracts and failure behavior.

## Explicit non-goals

- Reimplementing Python Factory's Polylith, dynamic discovery, agent runtime, UI, distributed executor, database adapters, or mesh protocol in this first slice.
- Executing arbitrary shell commands or overwriting arbitrary user paths.
- Choosing an LLM framework, async runtime, web framework, or CRDT/mesh implementation before the project contract is proven.

## Next layers

After the project contract is stable, add agent definitions and a local runtime; then workflow and evaluation; then edge and mesh adapters. Workflow remains a durable, domain-agnostic capability, but is not the first product slice.
