# Status-Only Capability Scaffolds

GitHub issue: #9

> **Superseded in part by [Issue #11](https://github.com/bannff/Rust-Factory/issues/11).**
> This spec was delivered as written, then revised. Requirement 1's 13-family
> enumeration no longer holds: 9 of those families own no package. Only
> `model-gateway`, `memory`, `sandbox`, and `observability`
> remain as status-only trees; `workspace-governance`, `identity`, `knowledge`,
> `verification`, `message-bus`, `cache`, `graph`, and `notification` are
> registry rows naming a future crate, and `tool-execution` folded into
> Sandbox. Requirements 2, 3, and 5 remain in force for any package that is
> status-only. Requirement 4's validator was generalized to cover every
> workspace package and to check the registry in both directions; it now lives
> at `scripts/validate_brick_registry.py`. The
> [Vision portfolio registry](../../steering/living-factory-vision.md#brick-portfolio-registry)
> and the [Canonical Brick Standard](../brick-standard/requirements.md) are
> authoritative.

Create the first mandatory, agent-maintainable status-only core scaffold rollout. This batch establishes fixed package paths and machine-readable metadata without inventing domain contracts or adapter behavior.

1. Add root workspace members `workspace-governance`, `identity`, `model-gateway`, `memory`, `knowledge`, `tool-execution`, `sandbox`, `verification`, `message-bus`, `cache`, `graph`, `observability`, and `notification`.
2. Every added package SHALL have `package.metadata.rust-factory` with its exact family name, `role = "core"`, and `status = "scaffolded"`; use workspace version/edition/license/rust-version, contain exactly `[lints]` with `workspace = true`, and declare no dependencies.
3. Every added package SHALL contain exactly `Cargo.toml` plus the mandatory status-only paths: `src/lib.rs`, `src/model.rs`, `src/validation.rs`, `src/error.rs`, `src/port.rs`, `src/service.rs`, and `tests/public_contract.rs`. No build script, binary, additional source/test file, or other package content is allowed. Leaf module/test files SHALL contain only blank lines and comments. `lib.rs` SHALL contain only crate documentation/comments and one exact private `mod <name>;` declaration for each reserved module; it SHALL expose no public semantic API or executable item and make no behavior, durability, security, tenancy, or framework claim.
4. Add a deterministic repository validator that reads Cargo metadata and package manifests to reject unknown Rust Factory roles/statuses, incomplete or expanded scaffold trees, non-status-only Rust source, Cargo build/target/features configuration, dependency tables, and family/status disagreement with the Living Factory Vision registry. It SHALL include deterministic negative self-tests and be invoked by the repository quality path without installing dependencies or accessing the network.
5. Existing packages are not reorganized or given metadata in this batch. Adapter infrastructure, composition bases, and optional domain packs do not receive new packages. No `*-memory`, `*-mcp`, `*-server`, vendor, or mesh package is created.

## Non-goals

No domain models, ports, cache/graph/message-bus behavior, framework/provider dependencies, MCP tools, storage/network access, Project Blueprint changes, or changes to existing MCP lifecycle ownership.
