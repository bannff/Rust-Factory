# Design

```text
agent-mcp ───────┐
workflow-mcp ────┼──> mcp-transport ───> rmcp / Tokio framing primitives
 evaluation-mcp ─┤
project-mcp ─────┘

all *-core crates: no dependency on mcp-transport, rmcp, Tokio, or futures
```

`mcp-transport` owns the reusable server-side boundary already proved by Agent, Workflow, and Evaluation MCP adapters. Its public surface is intentionally narrow:

```text
BoundedStdioTransport<R, W>::new(reader, writer)
MAX_MCP_STDIO_FRAME_BYTES
```

The transport is fixed to rmcp's server role and generic only over asynchronous reader/writer types for testability and composition. It owns an incremental decoder wrapper around rmcp's JSON-RPC codec. The wrapper checks payload bytes before the typed rmcp decoder: a complete line strips a single CR before measuring; without a newline, only the exact `64 KiB + CR` pending-CRLF state may exceed the payload count. The writer is serialized with a mutex and is dropped when framing becomes terminal.

MCP adapter libraries continue to expose their existing `serve_stdio` convenience helper in this extraction, now passing `BoundedStdioTransport::new(tokio::io::stdin(), tokio::io::stdout())`. The later server-composition-root spec owns moving startup/configuration out of those helpers; this transport extraction must not conflate that deployment concern.

The shared implementation adopts Agent's stricter incremental no-newline rule: only the exact `64 KiB + CR` pending-CRLF state can exceed the payload count before a newline. This replaces Evaluation's earlier permissive no-newline threshold and Workflow's codec-only transport that could not express exact-limit CRLF behavior.

## Migration

1. Create `mcp-transport` as a root workspace member with direct exact rmcp/futures/tokio-util/serde/serde_json dependencies, production Tokio limited to `io-util` and `sync`, and a separate exact Tokio dev-dependency enabling `macros`, `rt-multi-thread`, and `time` only for decoder/transport contract tests.
2. Replace each of Agent, Workflow, Evaluation local module imports with the shared crate and remove their duplicated modules/tests.
3. Replace Project MCP raw `rmcp::transport::stdio()` use with the shared transport; add its inherited framing coverage through the transport crate rather than duplicating tests.
4. Remove adapter-only `futures`/`tokio-util` dependencies that become unused after migration. Keep adapter `rmcp`, Tokio I/O, and existing service dependencies where their public `serve_stdio` helpers require them. Project result bounds are explicitly unchanged in this extraction.

## Verification

Transport contract tests use Tokio duplex and capture server writer bytes. They cover: valid LF/CRLF frames; exact 64 KiB payload in both forms; 64 KiB-plus-one LF/CRLF payload terminal close and successor suppression; 64 KiB-plus-one non-CR partial-frame immediate close; cancellation/re-poll of valid partial input; and cancellation/re-poll of the sole permitted `64 KiB + CR` pending-CRLF state. They also prove syntax/EOF JSON is ignored with later valid receipt, well-formed JSON-RPC data/shape failure emits only rmcp's generic invalid-request response, and any framing overflow/non-Serde failure writes zero response bytes before terminal close. A concurrent-send duplex test proves two send futures produce complete non-interleaved newline-delimited JSON-RPC frames, and a post-terminal `send` test proves writer closure returns a transport failure. Adapter regression tests preserve each MCP crate's existing public surface and Policy behavior. Cargo metadata/tree checks prove the dependency stays outside core crates.
