# Agent Effective Capability Ceiling

## Purpose

Provide the policy-neutral Agent library seam required for external grants to narrow a definition-derived runtime scope without allowing elevation.

`EffectiveCapabilityCeilingV1` is owned by `agent`: ordered tool IDs plus memory/knowledge/sandbox/communication booleans. `LocalAgentRuntime::invoke_with_ceiling` intersects it with the Agent definition before constructing `ResolvedCapabilityScope`, model request, or adapter requests. The intersection is canonical, bounded, digest-bound, and deny-by-default.

A ceiling may only remove definition permissions. Grant-disallowed tools and capabilities must not appear in the model-visible scope and must not reach ToolRegistry, MemoryStore, KnowledgeStore, or Sandbox. Existing `invoke` remains a compatibility wrapper using the definition’s full ceiling until Agent MCP is migrated.

Required tests: ordered/deduplicated intersection; no elevation; model scope excludes denied items; every denied capability fails before adapter call; scope digest changes with effective ceiling; compatibility invoke behavior remains unchanged.