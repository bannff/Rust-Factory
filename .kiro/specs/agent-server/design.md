# Design

```text
launch arguments                         versioned JSON config
--tenant-id / --principal-id              static local data only
              │                                      │
              ▼                                      ▼
ProcessTrustedContextSource                 AgentServerConfigV1
(fresh operation IDs)                              │
              └───────────┐              ┌─────────┘
                          ▼              ▼
              AgentDefinitionMcp::new(verified resolver)
                          │
                          ▼
               existing serve_stdio() → agent-mcp → mcp-transport → stdio
```

`agent-server` is a binary-only composition root. It owns process startup, bounded configuration decoding, launch-context validation, construction of concrete local adapters, and the Tokio runtime that drives `agent-mcp`. The server has no reusable domain contract: its only public executable behavior is the existing Agent MCP server surface.

`AgentServerConfigV1` is an internal, versioned serde type supplied exactly once through `--config-json`; MCP stdin is reserved exclusively for `serve_stdio()`. The raw UTF-8 argument is limited to 256 KiB before serde decoding. `deny_unknown_fields`, a closed version discriminator, and binary-private collection/string ceilings make it fail closed rather than silently accepting configuration that appears to affect identity, provider selection, tools, or authorization. Conversion reuses existing Agent/Policy constructors and limits, checks the additional bootstrap ceilings before constructing any adapter, and completes before Tokio/MCP serving begins.

`ProcessTrustedContextSource` is private to the binary. It receives validated tenant and principal values once from mandatory launcher arguments and creates a new `TrustedContextV1` per adapter operation. A synchronization-safe `AtomicU64` sequence assigns each `resolve` call one value; the source validates distinct `request-<sequence>` and `correlation-<sequence>` logical IDs from it. Atomic exhaustion or validation failure returns an error, so allocation never wraps or reuses an ID. Neither IDs nor identity are accepted through config or MCP parameters. This is a local trusted-launcher seam, not an authentication system: changing to OS-derived, remote, or tenant-boundary identity requires a dedicated process-boundary Policy specification.

The root uses existing deterministic adapters only:

* `InMemoryDefinitionStore` retains globally shared V1 definitions for this process only.
* `StaticReferenceCatalog`, `StaticModelProvider`, and `StaticKnowledgeStore` project config data deterministically.
* `FixedToolRegistry::default()` provides the current fixed local registry.
* `InMemoryMemoryStore::default()` is process-local.
* `DenySandbox` rejects sandbox execution.
* `StaticPolicyResolver` is initialized from static grants and denies unmatched tenant/principal/capability decisions.

The root passes those adapters plus the process context source into `AgentDefinitionMcp::new(...)` and delegates stdio lifecycle and bounded framing to the established `serve_stdio()` helper. It must not implement a duplicate router, policy gate, transport, or identity parsing path. This verifies adapter-library/binary-root separation without moving code into Agent core.

## Test strategy

Unit tests prove strict V1 config decoding: valid conversion, unknown fields, unsupported version, malformed nested values, and invalid static grants reject before server construction. Construction tests record that launch arguments map to the expected Policy IDs; source calls use those IDs and generate distinct operation request/correlation IDs. A default-deny test invokes the constructed verified resolver and proves no matching grant is denied. A component construction test exercises config-to-adapter conversion with all optional static data combinations.

A process-level stdio smoke test launches the binary with a minimal valid config and explicit launch identity, sends one valid bounded JSON-RPC MCP request, and observes the existing MCP response. A counterpart with an identity that has no matching configured grant observes the established denial projection and no domain-effect output. The test sends no identity or Policy decision data in MCP input. Existing `agent-mcp` tests remain the authority for full schema, transport, Policy-digest, and capability-ceiling matrices.

## Verification

Before implementation acceptance, run the narrow server-related suite:

```sh
cargo test -p agent-server -p agent -p agent-mcp -p policy-memory -p mcp-transport
```

Then complete QA, security, final architecture, and Rust SME gates, followed by `make check`. The README and Factory Blueprint are updated only if implementation demonstrates a durable composition convention beyond this single local binary.
