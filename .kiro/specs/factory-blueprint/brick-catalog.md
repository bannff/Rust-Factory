# Brick Catalog

## Implemented

| Brick | Responsibility | Current guarantee |
|---|---|---|
| Project | Blueprint → validated Rust workspace | deterministic planning; confined filesystem adapter; MCP |
| Agent | Definitions, Policy-gated MCP operations, local invocation, and Agent-owned tool/memory/sandbox orchestration | deterministic local adapters; consumes LLM Gateway and Knowledge inward; Knowledge policy/planning/preflight/events/accounting remain Agent-owned; definitions remain globally shared in V1 |
| Knowledge | Bounded synchronous retrieval through canonical `KnowledgeIndex`/`KnowledgeService` | framework-free core and std-only immutable static adapter implemented; Agent migration, QA/security, final Rust SME/meta-architecture reviews, status promotion to `implemented`, focused matrix, and `make check` complete with no Blocker or Required findings; only issue evidence, merge, and delivery pending; no MCP/settings/ingestion/async/ranking/vector/remote/persistence/lifecycle |
| Workflow | Lifecycle contract around an Agent attempt | process-local in-memory adapter; no recovery/cross-process cancellation |
| Evaluation | Immutable terminal-workflow evidence assessment | deterministic in-memory reader/store and Policy-compatible MCP; no workflow mutation |
| MCP transport | Shared server-side bounded stdio framing adapter | exact 64 KiB LF/CRLF ingress contract; adapter-only rmcp/Tokio dependencies; all four MCP surfaces migrated to the shared bounded transport and then had `serve_stdio()` deleted, so no library owns lifecycle and the transport awaits its first `projects/` binary (#17) |
| Adapter portfolio | Declarative selection and experiment doctrine, not a runtime registry | Blueprint-only; implementation deferred until a demonstrated project-planning consumer |

## Ownership and extraction

Agent currently owns `ToolRegistry`, `MemoryStore`, and `Sandbox`; Memory also has an implemented canonical brick but Agent has not migrated its provisional port, while Tool and Sandbox remain Agent-owned (`sandbox` is status-only). Knowledge is the completed narrow live-port extraction: one demonstrated direct consumer, an explicit issue #37 product mandate, pre-setup Rust SME approval, and an atomic one-way migration with no alias or compatibility facade moved ownership to Knowledge. Agent now depends inward on Knowledge-owned `KnowledgeIndex`/`KnowledgeService`.

Ordinarily, extract a narrowly named transport-independent capability only after at least two demonstrated direct consumers prove a stable need. The live-provisional-port exception above is narrow and does not authorize pre-consumer packages. Zero-consumer packages remain prohibited; Storage is the sole approved exception. No generic umbrella core exists.

`policy` owns trusted principal/tenant grants and execution-boundary decisions. Agent retains definition policy data; Agent, Workflow, and Evaluation MCP adapters inject verified Policy decisions before their authorized domain paths.

## Deferred

Persistent Workflow adapter: leases, recovery, cross-process cancellation, retries, and durable effect acknowledgement. Mesh/Edge: selected peer transport, encrypted messaging, offline recovery, and only explicitly replicated state. CRDTs never authorize or deliver consequential effects.
