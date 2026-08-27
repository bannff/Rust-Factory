# Tasks

GitHub issue: #4

- [x] Add `mcp-transport` as a root workspace member with direct exact rmcp/futures/tokio-util/serde/serde_json dependencies, production Tokio limited to `io-util` and `sync`, a separate exact Tokio dev-dependency for `macros`, `rt-multi-thread`, and `time`, and no core dependency edges.
- [x] Implement the canonical server-side bounded stdio transport and duplex contract tests for payload/CRLF/malformed/terminal/writer/cancellation behavior, including concurrent-send serialization and post-terminal send failure.
- [x] Migrate Agent, Workflow, Evaluation, and Project MCP adapters to path-depend on the shared transport; remove all duplicated local transport modules and Project raw stdio use.
- [x] Remove only migrated adapter dependencies that become unused; retain existing rmcp/Tokio service dependencies and leave Project output bounds unchanged.
- [x] Verify no core crate depends on `mcp-transport`, rmcp, Tokio, futures, tokio-util, serde, or serde_json because of this adapter extraction.
- [x] Preserve all MCP adapter tool schemas, trusted-context/Policy semantics, existing result projections (including Project's currently unbounded plan/generate projection), and public error behavior.
- [x] Run focused transport and migrated MCP tests, then QA, security, Rust SME, architecture, and `make check`.

## Acceptance evidence

- Rust SME approved the specification and final implementation, including exact production/dev dependency separation.
- QA approved the canonical framing migration and added clean-EOF writer closure/send-failure coverage.
- Security approved pre-deserialization bounds, malformed/error behavior, writer lifecycle, and core isolation.
- Architecture approved the adapter-only transport boundary and all-four MCP migration as scoped Issue #4 work.
- `make check` passed after the final dependency-feature narrowing.

Binary composition roots, remote clients, process-boundary Policy, edge topology, and async execution ports remain separate tracked work.
