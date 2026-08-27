# Tasks: Rust Factory Foundation

- [x] 1. Establish the Cargo workspace and workspace lint policy. The initial `factory-core` placeholder was removed in the focused no-consumer cleanup tracked by GitHub issue #2.
- [x] 2. Add shared Rust fundamentals, architecture, quality, and async guidance.
- [x] 3. Add the local and CI quality gates.
- [x] 4. Define the first product slice as an MCP-exposed project brick.
- [x] 5. Define versioned project-blueprint, validation-report, generation-plan, generated-file, project-target, and error contracts in `project`.
- [x] 6. Implement pure blueprint validation and deterministic generation planning for a one-crate library or binary workspace.
- [x] 7. Add focused tests for invalid names, validation findings, project targets, and plan determinism.
- [x] 8. Define a root-confined `ProjectWriter` port and implement a filesystem adapter that rejects traversal and overwrite.
- [x] 9. Add filesystem integration tests and verify generated fixture projects with `make check`.
- [x] 10. Add a bounded MCP adapter with `project_validate`, `project_plan`, and `project_generate` operations.
- [x] 11. Contract-test MCP tool names, generation input schema, and stable validation error mapping.
- [x] 12. Review the project public API: MCP injects the `ProjectWriter` port; the local adapter uses capability-confined I/O and atomic target reservation; MCP responses do not leak host paths or raw I/O errors.

## Validation matrix

Narrow command: `cargo test -p project -p project-fs -p project-mcp`. Required: validation/plan determinism, filesystem confinement, MCP schema, generated fixture. Conditional: proptest for name/path invariants; fuzz parser ingress. N/A: loom until shared-state adapter; canonical golden hashes (no canonical record contract).