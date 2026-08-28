# Design

```text
agent::mcp ───────┐
workflow::mcp ────┼──> mcp-transport ───> rmcp / Tokio framing primitives
 evaluation::mcp ─┤
project::mcp ─────┘

all capability library crates: no dependency on mcp-transport, rmcp, Tokio, or futures
```

`mcp-transport` owns the reusable server-side boundary already proved by Agent, Workflow, and Evaluation MCP adapters. Its public surface is intentionally narrow:

```text
BoundedStdioTransport<R, W>::new(reader, writer)
MAX_MCP_STDIO_FRAME_BYTES
```

The transport is fixed to rmcp's server role and generic only over asynchronous reader/writer types for testability and composition. It owns an incremental decoder wrapper around rmcp's JSON-RPC codec. The wrapper checks payload bytes before the typed rmcp decoder: a complete line strips a single CR before measuring; without a newline, only the exact `64 KiB + CR` pending-CRLF state may exceed the payload count. The writer is serialized with a mutex and is dropped when framing becomes terminal.

In this extraction the MCP surfaces kept a `serve_stdio` convenience helper passing `BoundedStdioTransport::new(tokio::io::stdin(), tokio::io::stdout())`. Those helpers were deleted when each brick became one crate, so transport binding now belongs solely to a composition-root binary.

The shared implementation adopts Agent's stricter incremental no-newline rule: only the exact `64 KiB + CR` pending-CRLF state can exceed the payload count before a newline. This replaces Evaluation's earlier permissive no-newline threshold and Workflow's codec-only transport that could not express exact-limit CRLF behavior.

## Migration

1. Create `mcp-transport` as a root workspace member with direct exact rmcp/futures/tokio-util/serde/serde_json dependencies, production Tokio limited to `io-util` and `sync`, and a separate exact Tokio dev-dependency enabling `macros`, `rt-multi-thread`, and `time` only for decoder/transport contract tests.
2. Replace each of Agent, Workflow, Evaluation local module imports with the shared crate and remove their duplicated modules/tests.
3. Replace Project MCP raw `rmcp::transport::stdio()` use with the shared transport; add its inherited framing coverage through the transport crate rather than duplicating tests.
4. Remove adapter-only `futures`/`tokio-util` dependencies that become unused after migration. Keep `rmcp` and existing service dependencies in the `mcp` modules; direct Tokio I/O left the bricks along with `serve_stdio`. Project result bounds are explicitly unchanged in this extraction.

## Verification

Transport contract tests use Tokio duplex and capture server writer bytes. They cover: valid LF/CRLF frames; exact 64 KiB payload in both forms; 64 KiB-plus-one LF/CRLF payload terminal close and successor suppression; 64 KiB-plus-one non-CR partial-frame immediate close; cancellation/re-poll of valid partial input; and cancellation/re-poll of the sole permitted `64 KiB + CR` pending-CRLF state. They also prove syntax/EOF JSON is ignored with later valid receipt, well-formed JSON-RPC data/shape failure emits only rmcp's generic invalid-request response, and any framing overflow/non-Serde failure writes zero response bytes before terminal close. A concurrent-send duplex test proves two send futures produce complete non-interleaved newline-delimited JSON-RPC frames, and a post-terminal `send` test proves writer closure returns a transport failure. Adapter regression tests preserve each MCP crate's existing public surface and Policy behavior. Cargo metadata/tree checks prove the dependency stays outside core crates.
