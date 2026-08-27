# Local Agent MCP Server

GitHub issue: #6

Add one thin local `agent-server` binary that composes the existing Agent MCP adapter with explicit, deterministic local dependencies. It proves the Factory composition-root pattern; it is not a new Agent runtime, transport, provider, persistence, or deployment framework.

1. `agent-server` SHALL be a root Cargo workspace member and binary package. Its production dependencies SHALL be exact-pinned path dependencies on `agent-mcp`, `agent-core`, `policy-core`, and `policy-memory` (all `version = "=0.1.0"`), plus exact `anyhow`, `serde` with derive, `serde_json`, and `tokio` with only `macros` and `rt-multi-thread`. It SHALL have no direct dependency on `mcp-transport`, `rmcp`, filesystem/network/config-framework crates, provider SDKs, or sandbox executors. No existing core or MCP-library dependency edge changes.
2. The binary SHALL accept exactly one `--config-json` launch argument, distinct from identity arguments, as the sole bootstrap source for a versioned `AgentServerConfigV1` JSON document. Its UTF-8 value SHALL be at most 256 KiB before deserialization; it SHALL be fully decoded and validated before Tokio/MCP serving begins. `serde(deny_unknown_fields)` and a closed version discriminator SHALL reject unknown fields and versions. No configuration is read from MCP stdin, the filesystem, environment, or network.
3. Static configuration contains only Agent builtins, static reference-catalog values (models, skills, steering, tools), one deterministic model response, optional static knowledge values, and static Policy grant records. Bootstrap validation SHALL enforce: at most 64 builtins; at most 64 entries in each catalog set, each reference at most 128 bytes; at most 16 knowledge values, each at most `agent_core::MAX_INPUT_BYTES`; model output at most 16 KiB; at most 16 static model tool calls, with each tool ID at most 128 bytes and input at most 4 KiB; and at most `policy_memory::MAX_STATIC_GRANT_RECORDS` grants, each using existing Policy ID/tool limits. Conversion SHALL reject over-limit, malformed, duplicate, or incompatible values before adapter construction. Configuration SHALL NOT contain credentials, provider configuration, arbitrary commands, filesystem paths, caller/request/correlation IDs, or identity.
4. The process SHALL accept exactly one `--tenant-id` and one `--principal-id` launch argument. A private `ProcessTrustedContextSource` SHALL validate them through existing Policy V1 newtypes and retain only valid launch-owned identity. On every `TrustedContextSource::resolve` invocation it SHALL atomically allocate one never-reused sequence number, format distinct grammar-valid `request-<sequence>` and `correlation-<sequence>` IDs, and fail closed when the sequence is exhausted or an ID cannot be validated. It SHALL not hardcode identity values, accept MCP-supplied identity, or obtain identity from configuration.
5. The composition root SHALL inject only existing static/local adapters: `InMemoryDefinitionStore`, `StaticReferenceCatalog`, `StaticModelProvider`, `FixedToolRegistry::default()`, `InMemoryMemoryStore::default()`, `StaticKnowledgeStore`, `DenySandbox`, and a `StaticPolicyResolver` built from configured grants. It SHALL construct `AgentDefinitionMcp` with its verified Policy resolver and invoke the existing `AgentDefinitionMcp::serve_stdio()` entry point. It SHALL not create a second MCP router or bind a raw rmcp/mcp-transport stdio transport.

  > **Unresolved conflict — settle before implementing.** Calling
  > `AgentDefinitionMcp::serve_stdio()` leaves stdio lifecycle ownership inside
  > the MCP *library*, which [Canonical Brick Standard](../brick-standard/requirements.md)
  > requirement 6 forbids: a `*-mcp` crate "SHALL NOT read `stdin`, write
  > `stdout`, call `serve_stdio`, construct `BoundedStdioTransport`, or choose
  > Tokio process lifecycle." `agent-mcp` is stamped
  > `status = "migration-pending"` to record that violation. Implementing this
  > requirement as written would make the workspace's first `role = "server"`
  > package a fresh instance of the forbidden pattern rather than the migration
  > that retires it. Either move the transport binding into the binary as part
  > of #6, or record an explicitly scoped and time-bound interim exception here
  > first.
6. The server SHALL preserve `agent-mcp` tool schemas, bounded stdio behavior, verified Policy decisions, and Agent capability-ceiling enforcement. Its static Policy resolver defaults to deny when no configured grant authorizes the operation. A denial reaches no Agent domain port through the existing adapter boundary.
7. Documentation and tests SHALL state the actual guarantees: configuration, policy grants, definitions, memory, knowledge, and execution state are process-local; static adapters are deterministic only; the trusted context is supplied by a local trusted launcher. V1 makes no OS-authentication, remote-client, tenant-isolated-definition, durable/recoverable, provider-backed, arbitrary-tool, or sandbox-execution claim.

## Non-goals

No changes to `agent-core`; no new public core port; no new MCP tool or MCP client; no Policy persistence, revocation, or cross-process distribution; no remote listener; no configuration file discovery; no environment-variable identity fallback; no credentials; no dynamic provider/tool/sandbox selection; no durable definitions, memory, knowledge, workflow, cancellation, retries, or shutdown protocol; and no edge or mesh topology.
