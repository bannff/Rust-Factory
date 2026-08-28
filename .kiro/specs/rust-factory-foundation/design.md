# Design: Rust Factory Foundation

## First vertical slice

The initial Factory brick is **project authoring**: a typed blueprint becomes a deterministic plan, then a safe generated Rust workspace. Its MCP adapter lets an external agent inspect and drive this capability.

```text
ProjectBlueprint
      │
      ▼
project core ──> ValidationReport + GenerationPlan
      │                              │
      │                              ├── MCP adapter: validate / plan / generate
      │                              └── filesystem adapter: safe materialization
```

- `crates/project` — blueprint, validation, planning, and writer-port contracts.
- `crates/project::fs` — root-confined filesystem materialization.
- `crates/project::mcp` — bounded MCP adapter using the official `rmcp` SDK.

The project core owns domain models, validation, template selection, and deterministic file content. The MCP and filesystem adapters depend on the core and own protocol, serialization, configured paths, and I/O. Dependencies always flow inward.

## Blueprint and plan

`ProjectBlueprint` is a versioned, declarative description of a generated Rust workspace. The first supported shape is intentionally narrow: workspace name, package/crate name, project kind (`library` or `binary`), license, and optional description. Future project kinds and features are typed data extensions, not ad hoc conditional branches.

`validate(blueprint)` returns a `ValidationReport` containing typed errors and warnings. `plan(blueprint)` returns an ordered collection of `GeneratedFile { relative_path, content }` plus the intended Cargo quality commands. Planning is pure and deterministic. `generate(plan, target)` materializes only a validated plan through a `ProjectWriter` port.

## Ports and operations

- `ProjectAuthor`: validates blueprints and creates generation plans.
- `ProjectWriter`: materializes a plan through a capability-confined target-root directory, returns a root-relative target identifier, and exposes no arbitrary-write operation.
- The MCP server accepts an injected `ProjectWriter` implementation; it depends only on core contracts and MCP/serialization libraries, never a concrete filesystem adapter.

The MCP adapter exposes only:

- `project_validate` — return the validation report for a blueprint;
- `project_plan` — return a dry-run plan without writing files;
- `project_generate` — materialize a valid plan in an approved target; and
- `project_verify` — reserved until a bounded quality-runner adapter is designed.

`project_generate` accepts only a root-relative target identifier. The capability filesystem adapter atomically reserves that target before writing and rejects paths that escape its opened root. MCP generation results return the target identifier and relative written paths; error responses use stable public codes and never disclose configured root paths, staging paths, or raw operating-system errors.

## Generated baseline

The first generated workspace includes a Cargo workspace manifest, one library or binary crate, workspace lint policy, `Makefile` quality commands, `.gitignore`, and a concise README. It does not add networked dependencies, an agent runtime, MCP server, database, or application framework.

## Validation strategy

Unit-test blueprint validation and plan determinism. Integration-test filesystem materialization, path traversal rejection, and overwrite refusal. Contract-test the MCP operation schemas and error mapping. Verify each generated fixture with the generated workspace's `make check` command.

## Follow-on architecture

Once the project contract is stable, add the `agent` brick and local Rust runtime, then workflow/evaluation, then edge/mesh adapters. Each follows the same core → adapter → bounded MCP-surface pattern described in the Living Factory Vision.
