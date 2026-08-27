# MCP Transport Extraction

GitHub issue: #4

Extract the demonstrated bounded server-side MCP stdio framing contract into an adapter-only `mcp-transport` crate. It is a transport utility for MCP adapter libraries, not a Factory core, runtime registry, binary, or deployment framework.

1. `mcp-transport` SHALL be a root workspace member and depend directly on exact-pinned `rmcp` with server/transport-I/O features, production `tokio = "=1.48.0"` with only `io-util` and `sync`, `futures = "=0.3.34"`, `tokio-util = "=0.7.19"` with `codec`, `serde = "=1.0.228"` without derive support, and `serde_json = "=1.0.145"`. Its exact `tokio = "=1.48.0"` dev-dependency SHALL enable `macros`, `rt-multi-thread`, and `time` solely for tests. No `*-core` crate may depend on it or on its framework dependencies. Only `agent-mcp`, `workflow-mcp`, `evaluation-mcp`, and `project-mcp` gain a path dependency on `mcp-transport`.
2. It SHALL expose `BoundedStdioTransport<R, W>` for the rmcp server role and `MAX_MCP_STDIO_FRAME_BYTES = 64 * 1024` as the maximum JSON-RPC payload bytes for one newline-delimited message. LF and an optional immediately preceding CR are framing delimiters and are excluded from the payload ceiling.
3. The transport SHALL bound incrementally before JSON-RPC or typed `Parameters<T>` deserialization. It accepts a payload of exactly 64 KiB framed with either LF or CRLF; it rejects a 64 KiB-plus-one payload, including a non-CR partial frame before any newline. The one allowed partial state above the ceiling is exactly 64 KiB payload plus a trailing CR awaiting LF.
4. An over-limit or non-Serde framing error SHALL terminally close the server transport without emitting a response or dispatching a queued successor frame. Syntax/EOF malformed JSON follows rmcp-compatible ignore behavior; well-formed data/shape errors may receive rmcp's generic invalid-request response. No behavior shall reveal adapter internals.
5. Valid partial input SHALL survive receive-future cancellation/re-poll without loss. Sends remain serialized through the transport writer. This V1 contract does not claim a complete outgoing JSON-RPC envelope ceiling. Agent, Workflow, and Evaluation retain their established 64 KiB serialized result projections. Project MCP's current plan/generate result projection remains unbounded and unchanged by this extraction; a Project output-bounds change requires a separate specification.
6. `agent-mcp`, `workflow-mcp`, `evaluation-mcp`, and `project-mcp` SHALL all construct this transport for their current `serve_stdio` helper. Their local `stdio_transport.rs` copies SHALL be removed; `project-mcp` SHALL stop using raw rmcp stdio directly.
7. Existing MCP tool schemas, trusted-context/Policy gates, public error projections, and core contracts remain unchanged. This extraction does not introduce binaries, remote clients, edge topology, async core ports, or a shared runtime abstraction.

## Non-goals

No client-role transport; socket/HTTP transport; dynamic plugin loading; process configuration; binary startup; remote inter-brick calls; policy forwarding; outbound full-frame guarantee; or changes to core dependency direction.
