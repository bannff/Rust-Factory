# Design

```text
host session adapter → TrustedContextV1
                         ↓
              PolicyResolver::authorize(capability)
                         ↓ verified Allow
agent-mcp compatibility adapter → AgentRegistry / LocalAgentRuntime
                                              ↓ invoke only
                              GrantV1 → EffectiveCapabilityCeilingV1
                                              ↓
                                  policy-neutral agent-core ports
```

`AgentPolicyContextResolver<T, P>` lives in `agent-mcp`. It owns a host `TrustedContextSource` and `PolicyResolver`, resolves trusted context once, authorizes one exact `CapabilityV1`, canonicalizes the returned Allow grant, and verifies the request-bound digest. Its authorization operation and resolved handoff are private. `AgentDefinitionMcp::new` accepts only this verified resolver, preventing an embedding caller from fabricating a tenant or Allow-looking digest.

Every handler first performs its bounded local validation. The generated MCP schema rejects every caller-supplied identity or policy field—including the zero-argument list tool—before handler dispatch. Only then may a handler call the resolver; only an Allow may access a registry, catalog, store, runtime, or capability port. The resolver returns an adapter-local outcome: source/context/canonicalization/digest-verification failures are rendered exactly as `operation_failed`, while deny is rendered exactly as `not_found`. It never uses `DefinitionError::AdapterFailure` for policy-gate failures, preserving the distinct Agent core `adapter_failure` contract after an Allow reaches the domain path. Validate/register retain their existing global definition behavior after authorization. Get/list expose existing global V1 definition projections after authorization. Agent Policy V1 gates access; it does not assert tenant ownership or hide globally shared definitions.

For invocation, the adapter converts the verified canonical grant to `EffectiveCapabilityCeilingV1` and calls `LocalAgentRuntime::invoke_with_ceiling`. Agent core intersects that ceiling with definition policy before resolving tools or building `ModelRequest.capability_scope`; model-requested denied capabilities fail before their ports. The adapter never passes trusted context or Policy decision data into Agent core and does not persist it.

## Bounded stdio transport

`agent-mcp` owns a private `BoundedStdioTransport`, based on the accepted Evaluation framing semantics: an adapter-owned wrapper accommodates rmcp’s CRLF accounting while enforcing a 64 KiB payload ceiling before its typed decoder. It accepts exact-limit LF/CRLF frames, preserves in-limit partial state across receive cancellation, and closes terminally on oversize input before dispatch. This is copied as a bounded adapter seam, not elevated to shared infrastructure.

## Test strategy

Recording source, Policy, registry/catalog/store, model, tool, memory, knowledge, and sandbox adapters prove each exact operation, pre-domain deny/failure behavior, and no leaks. A five-operation schema matrix injects each prohibited `tenant_id`, `principal_id`, `request_id`, `correlation_id`, grant, decision-digest, and ceiling field—including list—and proves parameter rejection before source/Policy/domain access. Policy-gate source/canonicalization/digest failures render exactly `operation_failed`; deny renders exactly `not_found`; post-Allow Agent core errors retain their existing public mapping. Invoke composition tests use the actual `LocalAgentRuntime` to prove grant-denied definition capabilities are excluded from model scope and cannot reach external-effect ports. Transport tests use Tokio duplex for LF/CRLF boundary, fragmented oversize, terminal successor suppression, and cancelled partial input. Agent core direct `invoke` remains covered as a full-definition compatibility path.
