# Agent MCP Policy Compatibility

Migrate Agent MCP to host-derived trusted context and Policy V1 decisions while preserving Agent core’s framework- and Policy-free contracts and V1 globally shared definition semantics.

1. Only `agent-mcp` gains adapter-facing dependencies on `policy` and its bounded-transport support. `agent` remains independent of Policy, MCP transport, Tokio, and identity types.
2. `agent-mcp` SHALL receive host-owned `TrustedContextV1` through a `TrustedContextSource` and `PolicyResolver` through a verified compatibility resolver. MCP inputs never establish identity and SHALL reject tenant, principal, request, correlation, grant, digest, or ceiling fields.
3. After bounded syntactic and semantic validation but before registry, reference-catalog, definition-store, model, tool, memory, knowledge, or sandbox access, each operation SHALL authorize exactly one closed capability: validate → `AgentDefinitionValidate`; get → `AgentDefinitionGet`; list → `AgentDefinitionList`; register → `AgentDefinitionRegister`; invoke → `AgentInvoke`.
4. The resolver SHALL canonicalize an Allow grant and recompute its request-bound decision digest. The `agent-mcp` adapter SHALL map host-source failure, trusted-context conversion failure, grant canonicalization failure, and tampered Allow digest to exactly `operation_failed`; it SHALL map deny to exactly `not_found`. These mappings are adapter-local and SHALL NOT reuse `agent::DefinitionError::AdapterFailure`, whose existing public code is `adapter_failure`. Every failure/deny path makes zero applicable domain-port calls.
5. `agent_definition_validate` and `agent_definition_register` validate the complete bounded definition before Policy, then may access the injected registry/catalog/store only after Allow. `get` validates the ID and `invoke` validates the ID and `agent::MAX_INPUT_BYTES` input bound before Policy. Invalid or oversized input makes zero source, Policy, and domain calls.
6. For `AgentInvoke` only, the adapter converts the verified canonical `GrantV1` field-for-field to `EffectiveCapabilityCeilingV1` and invokes `LocalAgentRuntime::invoke_with_ceiling`. A grant can only narrow the resolved definition scope; denied capabilities must be absent from model scope and rejected before the corresponding adapter call.
7. V1 definitions remain globally shared: Policy gates access to get/list/register/invoke but does not make the injected `DefinitionStore` tenant-private or conceal definitions by tenant. A tenant-scoped definition store is a separate migration.
8. The public `AgentDefinitionMcp` constructor changes to require the verified resolver. The legacy unprotected constructor/context seam is removed. Existing five tool names, closed schemas, safe public projections, and Agent core public contracts remain compatible.

## MCP schema and error boundary

For every one of the five MCP operations—including zero-argument `agent_definition_list`—the adapter SHALL reject each caller-supplied `tenant_id`, `principal_id`, `request_id`, `correlation_id`, `grant`, `decision_digest`, and `effective_capability_ceiling` field during MCP schema/parameter processing. A recording test matrix SHALL prove this rejection reaches no trusted-context source, Policy resolver, registry, catalog, store, model, tool, memory, knowledge, or sandbox port.

The private resolver returns an adapter-local policy-gate outcome, not `DefinitionError`. Handlers map source/context/canonicalization/digest-verification failure to exactly `operation_failed` and deny to exactly `not_found`; existing Agent core errors retain their current stable public codes after an Allow reaches the corresponding domain path.

## Bounded MCP ingress

`agent-mcp` SHALL replace direct `rmcp::transport::stdio()` with an adapter-private bounded newline-delimited JSON-RPC transport. A complete payload, excluding LF and optional CR delimiter, is at most 64 KiB. It must bound incrementally before JSON-RPC/`Parameters<T>` deserialization; retain valid partial input across cancelled receives; accept exact-limit LF and CRLF payloads; and terminally close without response on a 64 KiB-plus-one payload. An oversized frame reaches no source, Policy, registry, catalog, store, model, tool, memory, knowledge, or sandbox port.

The transport remains private to `agent-mcp`. Do not extract a shared MCP transport utility or introduce runtime framework selection in this migration.

## Non-goals

No Policy dependency in Agent core; no decision/grant persistence; no tenant-private definitions; no Policy MCP; no provider selection; no durable attempt/retry/cancellation model; no experiment or promotion behavior; and no change to direct `LocalAgentRuntime::invoke` compatibility behavior for non-MCP callers.
