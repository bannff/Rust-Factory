# Brick Catalog

## Implemented

| Brick | Responsibility | Current guarantee |
|---|---|---|
| Project | Blueprint → validated Rust workspace | deterministic planning; confined filesystem adapter; MCP |
| Agent | Definitions, Policy-gated MCP operations, local invocation, tool/memory/knowledge/sandbox execution ports | deterministic local adapters; Agent MCP Policy compatibility accepted; definitions remain globally shared in V1 |
| Workflow | Lifecycle contract around an Agent attempt | process-local in-memory adapter; no recovery/cross-process cancellation |
| Evaluation | Immutable terminal-workflow evidence assessment | deterministic in-memory reader/store and Policy-compatible MCP; no workflow mutation |
| MCP transport | Shared server-side bounded stdio framing adapter | exact 64 KiB LF/CRLF ingress contract; adapter-only rmcp/Tokio dependencies; all four existing MCP libraries migrated to the shared bounded transport, while `serve_stdio()` lifecycle ownership remains pending server migrations |
| Adapter portfolio | Declarative selection and experiment doctrine, not a runtime registry | Blueprint-only; implementation deferred until a demonstrated project-planning consumer |

## Ownership and extraction

Agent currently owns `ToolRegistry`, `MemoryStore`, `KnowledgeStore`, and `Sandbox` contracts because it is their sole consumer. Do **not** scaffold duplicate capability cores. Extract one only when a second brick requires it: the new capability core becomes canonical owner; `agent-core` consumes it through a one-way dependency; capability core never depends on Agent; compatibility facade/deprecation is specified in that extraction spec.

No generic umbrella core exists. Extract a narrowly named, transport-independent capability core only when a stable shared contract has at least two consumers; the new core is canonical owner, existing consumers depend inward on it, it never depends on a brick or adapter, and any compatibility facade/deprecation is specified by that extraction.

`policy-core` owns trusted principal/tenant grants and execution-boundary decisions. Agent retains definition policy data; Agent, Workflow, and Evaluation MCP adapters inject verified Policy decisions before their authorized domain paths.

## Deferred

Persistent Workflow adapter: leases, recovery, cross-process cancellation, retries, and durable effect acknowledgement. Mesh/Edge: selected peer transport, encrypted messaging, offline recovery, and only explicitly replicated state. CRDTs never authorize or deliver consequential effects.
