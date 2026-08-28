# Policy

## Purpose

Policy is the canonical trusted-context and capability-grant decision layer. It authorizes a trusted principal to use a bounded Factory capability; it does not authenticate tokens, mutate agent definitions, choose providers, or execute side effects.

## V1 requirements

1. `policy` owns validated `TenantId`, `PrincipalId`, `RequestId`, `CorrelationId`, `TrustedContextV1`, closed `CapabilityV1`, `GrantV1`, `AuthorizationRequestV1`, `AuthorizationDecisionV1`, errors, and core traits.
2. A trusted embedding/session adapter alone resolves `TrustedContextV1`; MCP input, workflow input, model output, stored data, and agent definitions never establish identity.
3. Closed capabilities cover existing controls: Agent definition validate/get/list/register, Agent invoke, Workflow validate/start/get/list/cancel, Evaluation validate/evaluate/get, Memory remember/recall/search/forget/status, and Observability telemetry query/status. The unit of granularity is **one capability per MCP tool**, deliberately, so a grant can permit reading without permitting mutation — `memory_forget` is an irreversible delete and collapsing it with `memory_recall` would be indefensible. A flat closed enum is also what makes requirement 4's deny-by-default on an unknown capability mechanical. Variant count is not a reason to restructure; the trigger would be a family needing a **grant shape** of its own, at which point a nested `CapabilityV1::Memory(MemoryCapabilityV1)` earns itself and `GrantV1`'s flat booleans become the more pressing problem. Adding a variant is a permanent wire commitment: `as_str()` is length-prefixed into the decision digest.
4. `PolicyResolver` returns allow or deny plus a canonical effective grant. Unknown principal, tenant, capability, malformed context, or resolver failure deny by default.
5. An effective Agent grant may intersect allowed tool IDs and capability booleans; it cannot add a capability absent from the Agent definition.
6. `policy::memory` is deterministic process-local static grants only. It makes no persistence, revocation propagation, token, delegation, or cross-process claim.
7. `policy` SHALL expose no MCP surface. It decides what an agent is permitted to do, so any agent-facing tool is a privilege-escalation seam: `AuthorizationRequestV1` carries trusted context, so authorizing through caller input would let a caller supply its own identity, which Canonical Brick Standard requirement 7 forbids. A caller-relative inspection tool was considered and rejected, not deferred: a capability list is a compile-time constant already implied by each brick's own tool schema, and a grant digest over caller-supplied input is authoritative evidence of nothing.

## Non-goals

OAuth/OIDC, credentials, token parsing, identity providers, durable policy administration, wildcard grants, impersonation, delegated authorization, network checks, provider rules, or authorization of consequential external effects.

## V1 scope decisions

Policy V1 authorizes Agent, Workflow, and Evaluation MCP operations. Project MCP remains intentionally unauthorised until Project receives a trusted-context migration spec; this is a deliberate temporary boundary, not an implicit allow.

Agent definitions are globally shared Factory definitions in V1. Policy gates operation access but does not claim tenant-private definition visibility; tenant-scoped DefinitionStore is a later breaking migration.

Before Agent MCP migration, `agent` SHALL introduce a policy-neutral `EffectiveCapabilityCeilingV1` invocation input. It is the intersection of definition scope and grant: ordered allowed tools plus memory/knowledge/sandbox/communication booleans. The runtime constructs its advertised/effective scope from that ceiling and must deny any definition-allowed but ceiling-disallowed capability before model advertisement or adapter invocation.
