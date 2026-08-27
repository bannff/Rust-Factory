# Design

`agent` defines `EffectiveCapabilityCeilingV1` and a validated intersection function. It depends on no Policy crate. Policy later converts an allowed `GrantV1` into this Agent library input at the MCP compatibility adapter boundary. This preserves one-way dependencies: `policy` never depends on `agent`; `agent-mcp` may depend on both adapter-facing contracts.

The ceiling is not an identity/authorization model. It is an explicit execution-boundary restriction with no wildcard or absent-list semantics. The model receives only the intersected `ResolvedCapabilityScope`; all runtime branches reuse that scope for tool/memory/knowledge/sandbox enforcement.