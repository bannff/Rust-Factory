
# Workflow evaluation demo

A deterministic standalone composition proving the local Agent → Workflow → Evaluation → Observability path, exposed as one unified `factory_*` MCP server process over stdio.

## Running

This is a long-running MCP server, not a one-shot binary. It reads newline-delimited JSON-RPC requests from stdin and writes responses to stdout until the input stream closes:

```sh
cargo run --locked
```

Drive it with any MCP client that speaks stdio transport (for example an editor's MCP integration, or `mcp-cli`). There are no command-line arguments, environment variables, or other external inputs; the server accepts only JSON-RPC requests on stdin.

## Tools

The server registers exactly four tools, all under the reserved `factory_` namespace:

- `factory_capabilities` — introspection only. Returns the server name, version, and the exact list of registered tool names. No secrets, grants, or identity are projected.
- `factory_run_demo` — runs one Agent → Workflow → Evaluation → Observability cycle using the fixed demo tenant/agent/workflow identity baked into the composition at startup, under a deny-all capability ceiling. Returns `run_id`, `evidence_digest`, `result_digest`, and `verdict`. Idempotent: repeated calls replay the same stored run and evaluation result.
- `factory_get_run` — tenant-fixed lookup of a previously run workflow by `run_id`. Returns `status`, `terminal_reason`, and `output` on success, or `{"error":"not_found"}` for a missing run or a run belonging to a different tenant. Never synthesizes a result.
- `factory_query_telemetry` — bounded tenant-fixed telemetry query. Returns only the allowlisted attributes already emitted by the demo cycle (`workflow_run_id`, `evidence_digest`, `result_digest`, `verdict`); `limit` must be between 1 and 50.

### Example JSON-RPC payloads

Initialize, then list tools:

```json
{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2024-11-05","capabilities":{},"clientInfo":{"name":"example-client","version":"0.0.1"}}}
{"jsonrpc":"2.0","method":"notifications/initialized"}
{"jsonrpc":"2.0","id":2,"method":"tools/list"}
```

Run the demo cycle:

```json
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"factory_run_demo","arguments":{}}}
```

Look up the resulting run (substitute the `run_id` returned above):

```json
{"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"factory_get_run","arguments":{"run_id":"run-..."}}}
```

Query telemetry:

```json
{"jsonrpc":"2.0","id":5,"method":"tools/call","params":{"name":"factory_query_telemetry","arguments":{"limit":1}}}
```

## Guarantees and limits

This is a process-local, in-memory experiment. Workflow state, evaluation results, cancellation registrations, and telemetry are non-durable and disappear at process exit. It provides no restart recovery, cross-process coordination, remote cancellation acknowledgement, persistent audit trail, or production security/availability guarantee. The sandbox adapter always rejects execution and performs no effects.

Tenant, principal, request, and correlation identity are fixed constants baked into the composition at startup (see `src/composition.rs`). No tool input field ever supplies identity, tenant, or grants — `factory_get_run` and `factory_query_telemetry` only accept a bounded lookup id or query limit, both validated and bounded before use. `factory_capabilities` projects no secrets.

This harness is not a production deployment.
