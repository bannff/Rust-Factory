# Brick Catalog

## Implemented

| Brick | Responsibility | Current guarantee |
|---|---|---|
| Project | Blueprint → validated Rust workspace | deterministic planning; confined filesystem adapter; MCP |
| Agent | Definitions, policy, local invocation, tool/memory/knowledge/sandbox execution ports | deterministic local adapters; MCP |
| Workflow | Lifecycle contract around an Agent attempt | process-local in-memory adapter; no recovery/cross-process cancellation |
| Evaluation | Immutable terminal-workflow evidence assessment | deterministic in-memory reader/store and MCP; Policy MCP compatibility is the next migration |
| Adapter portfolio | Declarative selection and experiment doctrine, not a runtime registry | Blueprint-only; implementation deferred until a demonstrated project-planning consumer |

## Ownership and extraction

Agent currently owns `ToolRegistry`, `MemoryStore`, `KnowledgeStore`, and `Sandbox` contracts because it is their sole consumer. Do **not** scaffold duplicate capability cores. Extract one only when a second brick requires it: the new capability core becomes canonical owner; `agent-core` consumes it through a one-way dependency; capability core never depends on Agent; compatibility facade/deprecation is specified in that extraction spec.

`factory-core` is a reserved empty placeholder with no dependents. It SHALL NOT accumulate generic infrastructure. Retain it only until a demonstrated, transport-independent Factory-wide contract has at least two consumers; otherwise remove it in a focused cleanup change.

Policy is distinct: `policy-core` will own trusted principal/tenant grants and execution-boundary decisions. Agent retains definition policy data; adapters inject Policy decisions before effects.

## Deferred

Persistent Workflow adapter: leases, recovery, cross-process cancellation, retries, and durable effect acknowledgement. Mesh/Edge: selected peer transport, encrypted messaging, offline recovery, and only explicitly replicated state. CRDTs never authorize or deliver consequential effects.
