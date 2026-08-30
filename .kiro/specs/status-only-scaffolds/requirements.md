# Status-Only Capability Scaffolds

GitHub issue: #9

> **Superseded in part by [Issue #11](https://github.com/bannff/Rust-Factory/issues/11), [Issue #34](https://github.com/bannff/Rust-Factory/issues/34), and [Issue #37](https://github.com/bannff/Rust-Factory/issues/37).**
> Knowledge is no longer roadmap-only or status-only: issue #37 owns a real implemented `knowledge` brick. Its implementation, atomic Agent migration, Workflow fixture migration, adversarial QA, security review, final Rust SME/meta-architecture reviews, status promotion to `implemented`, focused Rust 1.88 validation, and `make check` are complete with no Blocker or Required findings. Only issue evidence, merge, and delivery remain pending; no merge or issue closure is claimed.
> This spec records the Issue #9 scaffold rollout as delivered, then revised.
> Requirement 1's 13-family enumeration no longer holds: 10 of those families
> own no package. Only `sandbox` remains a status-only tree; the former
> `model-gateway` tree was renamed and replaced by the implemented
> `llm-gateway` brick under Issue #34. Final Rust SME and meta-architect
> gates approved with no Blocker or Required findings remaining; only merge and
> issue delivery remain. There is no current implementation-pending scaffold
> claim. `workspace-governance`,
> `identity`, `verification`, `message-bus`, `cache`, `graph`, and `notification` remain roadmap items tracked in GitHub issues and GitHub Projects. `memory` and `observability` are implemented bricks, Knowledge is implemented under issue #37 with final reviews, status promotion, and `make check` complete and only issue evidence/merge/delivery pending, and `tool-execution` folded into Sandbox. Requirements 2, 3, and 5
> remain in force for any package that is status-only. Requirement 4's
> validator was generalized to cover every workspace package and reconcile
> workspace members, on-disk package directories, Cargo metadata, and package
> manifests; it now lives at `scripts/validate_brick_registry.py`. GitHub
> issues and GitHub Projects are authoritative for the capability roadmap and
> taxonomy; the [Canonical Brick Standard](../brick-standard/requirements.md)
> is authoritative for scaffold and brick structure.

Create the first mandatory, agent-maintainable status-only core scaffold rollout. This batch establishes fixed package paths and machine-readable metadata without inventing domain contracts or adapter behavior.

1. Add root workspace members `workspace-governance`, `identity`, `model-gateway`, `memory`, `knowledge`, `tool-execution`, `sandbox`, `verification`, `message-bus`, `cache`, `graph`, `observability`, and `notification`.
2. Every added package SHALL have `package.metadata.rust-factory` with its exact family name, `role = "core"`, and `status = "scaffolded"`; use workspace version/edition/license/rust-version, contain exactly `[lints]` with `workspace = true`, and declare no dependencies.
3. Every added package SHALL contain exactly `Cargo.toml` plus the mandatory status-only paths: `src/lib.rs`, `src/model.rs`, `src/validation.rs`, `src/error.rs`, `src/port.rs`, `src/service.rs`, and `tests/public_contract.rs`. No build script, binary, additional source/test file, or other package content is allowed. Leaf module/test files SHALL contain only blank lines and comments. `lib.rs` SHALL contain only crate documentation/comments and one exact private `mod <name>;` declaration for each reserved module; it SHALL expose no public semantic API or executable item and make no behavior, durability, security, tenancy, or framework claim.
4. Add a deterministic repository validator that reads Cargo metadata and package manifests to reject unknown Rust Factory roles/statuses, incomplete or expanded scaffold trees, non-status-only Rust source, Cargo build/target/features configuration, dependency tables, and disagreement among root workspace members, package directories, Cargo metadata, and package manifests. The validator SHALL NOT maintain or cross-check an in-repository roadmap: capability roadmap and taxonomy are authoritative in GitHub issues and GitHub Projects, while structural requirements are authoritative in the Canonical Brick Standard. It SHALL include deterministic negative self-tests and be invoked by the repository quality path without installing dependencies or accessing the network.
5. Existing packages are not reorganized or given metadata in this batch. Adapter infrastructure, composition bases, and optional domain packs do not receive new packages. No `*-memory`, `*-mcp`, `*-server`, vendor, or mesh package is created.

## Non-goals

No domain models, ports, cache/graph/message-bus behavior, framework/provider dependencies, MCP tools, storage/network access, Project Blueprint changes, or changes to existing MCP lifecycle ownership.
