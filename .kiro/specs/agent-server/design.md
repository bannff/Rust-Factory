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
               binary binds BoundedStdioTransport → agent::mcp → stdio
```

`agent-server` is a binary-only composition root. It owns process startup, bounded configuration decoding, launch-context validation, construction of concrete local adapters, the Tokio runtime that drives `agent::mcp`, and Agent's required `InvocationControlSource`. For every invocation it owns one stable idempotency key, a process-local cancellation source, and deadline wake mechanics tied to one fixed absolute deadline. Agent and LLM Gateway only borrow those controls; neither creates a timer, runtime, or lifecycle.

`AgentServerConfigV1` is an internal, versioned serde type supplied exactly once through `--config-json`; MCP stdin is reserved exclusively for the transport the binary binds. The raw UTF-8 argument is limited to 256 KiB before serde decoding. `deny_unknown_fields`, a closed version discriminator, and binary-private collection/string ceilings make it fail closed rather than silently accepting configuration that appears to affect identity, provider selection, tools, or authorization. Knowledge entries are scoped records with exactly `tenant_id`, `namespace`, `document_id`, and `text`; conversion uses Knowledge's validated IDs and document constructor, including 128-byte identifiers, nonempty 16-KiB text, 10,000-document, 64-MiB aggregate-text, and duplicate scoped-key limits. Conversion reuses existing Agent/Knowledge/Policy constructors and limits, checks the additional bootstrap ceilings before constructing any adapter, and completes before Tokio/MCP serving begins.

`ProcessTrustedContextSource` is private to the binary. It receives validated tenant and principal values once from mandatory launcher arguments and creates a new `TrustedContextV1` per adapter operation. A synchronization-safe `AtomicU64` sequence assigns each `resolve` call one value; the source validates distinct `request-<sequence>` and `correlation-<sequence>` logical IDs from it. Atomic exhaustion or validation failure returns an error, so allocation never wraps or reuses an ID. Neither IDs nor identity are accepted through config or MCP parameters. This is a local trusted-launcher seam, not an authentication system: changing to OS-derived, remote, or tenant-boundary identity requires a dedicated process-boundary Policy specification.

The root uses existing deterministic adapters only:

* `InMemoryDefinitionStore` retains globally shared V1 definitions for this process only. A validated definition selects the Knowledge namespace; caller/model/tool input cannot choose it. Retrieval filters documents by the launch-derived tenant and definition namespace; Policy/Agent admits the launch-derived principal, and configured grants must authorize every reachable definition namespace. Global definition visibility neither globalizes the corpus nor authorizes cross-tenant data; tenant-private definitions and principal-partitioned corpora are deferred.
* `StaticReferenceCatalog`, `llm_gateway::r#static::StaticProvider`, and `knowledge::r#static::StaticKnowledgeIndex` project config data deterministically. The server depends directly on `llm-gateway` and `knowledge` with `static`, then injects the Knowledge index into Agent's `KnowledgeIndex` dependency for use through `KnowledgeService`.
* `FixedToolRegistry::default()` provides the current fixed local registry.
* `InMemoryMemoryStore::default()` is process-local.
* `DenySandbox` rejects sandbox execution.
* `StaticPolicyResolver` is initialized from static grants and denies unmatched tenant/principal/capability decisions.

The root passes those adapters plus the process context source and a private `InvocationControlSource` into `AgentDefinitionMcp::new(...)`. The control source creates one owned request-scoped bundle with a stable `llm_gateway::IdempotencyKey`, cancellation signal, and runtime-backed deadline signal; cancellation/deadline wake pending work without moving runtime or timer ownership into a library. The root owns stdio lifecycle itself: it constructs `mcp_transport::BoundedStdioTransport` and drives `serve(...).waiting()`. The `serve_stdio()` helper it previously delegated to has been deleted from every brick, because a library owning process lifecycle violates requirement 6. It must not implement a duplicate router, policy gate, transport, or identity parsing path. This verifies adapter-library/binary-root separation without moving code into Agent core.

## Test strategy

Unit tests prove strict V1 config decoding: valid conversion, unknown fields, unsupported version, malformed nested values, and invalid static grants reject before server construction. Construction tests record that launch arguments map to the expected Policy IDs; source calls use those IDs and generate distinct operation request/correlation IDs. Control-source tests prove each invocation receives one stable key, cancellation and deadline wake pending work, and controls are not created before Policy allows invocation. A default-deny test invokes the constructed verified resolver and proves no matching grant is denied. A component construction test exercises config-to-adapter conversion with all optional static data combinations.

A process-level stdio smoke test launches the binary with a minimal valid config and explicit launch identity, sends one valid bounded JSON-RPC MCP request, and observes the existing MCP response. A counterpart with an identity that has no matching configured grant observes the established denial projection and no domain-effect output. The test sends no identity or Policy decision data in MCP input. Existing `agent::mcp` tests remain the authority for full schema, transport, Policy-digest, and capability-ceiling matrices.

## Verification

Before implementation acceptance, run the narrow server-related suite:

```sh
cargo test -p agent-server -p agent -p knowledge -p policy -p llm-gateway -p mcp-transport \
  --features agent/mcp,knowledge/static,policy/memory,llm-gateway/static
```

Then complete QA, security, final architecture, and Rust SME gates, followed by `make check`. The README and Factory Blueprint are updated only if implementation demonstrates a durable composition convention beyond this single local binary.
