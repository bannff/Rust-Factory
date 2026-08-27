# Policy Design

```text
policy-core
  TrustedContextV1 + CapabilityV1 + GrantV1 + Decision
  PolicyResolver trait
          ↑
policy-memory      static deterministic principal/tenant grants
          ↑
policy-mcp         optional caller-relative policy_check
          ↑
existing MCP adapters consume PolicyResolver through compatibility adapters
```

`policy-core` has no dependency on Agent, Workflow, Evaluation, MCP, storage, or identity frameworks. Existing brick MCP adapters inject `PolicyResolver`, derive context from a trusted host resolver, and authorize before resource/catalog/store access. Agent definition policy remains configuration; Policy grant intersects its resolved capability scope at the execution boundary.

## Contract

`TrustedContextV1` contains tenant, principal, request, and correlation IDs using the existing stable logical-ID grammar. `AuthorizationRequestV1` contains trusted context and one closed capability. `AuthorizationDecisionV1` contains `Allow { effective_grant, decision_digest }` or `Deny { safe_reason }`.

`GrantV1` is a closed data record: an always-present ordered tool allowlist (empty means no tools), and booleans for memory/knowledge/sandbox/communication. Canonical grant bytes use the exact V1 grant encoding below; grants are bounded, deduplicated, and cannot elevate absent Agent definition permissions.

## Migration

Phase 1: introduce Policy with static adapter/tests only. Phase 2: `workflow-mcp::RequestContextResolver` becomes a compatibility adapter over Policy context and decisions; every workflow operation authorizes before catalog/store/invoker access. Phase 3: replace Evaluation’s duplicate trusted context resolver. Phase 4: inject Policy into Agent MCP; agent definitions remain shared/global until a separately specified tenant-scoped DefinitionStore migration. No core crate depends on Policy during phase 1; MCP adapters depend on Policy only through adapters/constructors.

## Canonical decision encoding

`decision_digest` is request-bound evidence, not a reusable grant digest. Canonical bytes are V1 length-prefixed UTF-8 fields in this exact order: domain tag `policy-decision-v1`; tenant ID; principal ID; request ID; correlation ID; closed capability lowercase name; allow/deny enum; tool count and ordered tool IDs; memory/knowledge/sandbox/communication booleans encoded `true|false`. Empty tool sets encode count `0`; there are no absent fields. SHA-256 of these bytes is the decision digest. `GrantV1` uses the same grant suffix and has its own `grant_digest`; golden vectors are shared by Policy core and consuming adapter tests.

## Compatibility adapter contract

A host-owned `TrustedContextSource` resolves all four trusted IDs. An adapter calls `PolicyResolver::authorize(context, capability)` after bounded syntactic request validation but before every catalog, registry, reader, store, runtime, or invoker access. Deny maps to `not_found` for tenant/resource operations and `permission_denied` only for caller-relative policy inspection. Each migrated MCP operation requires a test proving resolver failure/deny makes zero domain-port calls. Workflow uses a compatibility `RequestContextResolver` backed by Policy; Evaluation replaces its duplicate resolver; Agent adds the same host context + resolver while retaining global definition visibility.

## Exact V1 decision/grant wire format

Every field is UTF-8 encoded as ASCII decimal byte length, `:`, bytes, `\n`; list counts are ASCII decimal fields under the same encoding. Booleans are exactly `true` or `false`. The allow record is: `policy-decision-v1`, tenant, principal, request, correlation, capability, `allow`, `grant-present`, tool count, ordered tool IDs, memory, knowledge, sandbox, communication. The deny record is: `policy-decision-v1`, tenant, principal, request, correlation, capability, `deny`, safe deny reason, `grant-absent`. A V1 effective grant always has a tool list: empty means **no tools**, never unrestricted. SHA-256 covers the complete record. `grant_digest` uses `policy-grant-v1`, tool count/tool IDs, and booleans only; `decision_digest` is request-bound evidence. Fixed golden vectors are mandatory.