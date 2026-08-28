# Design: Agent Definition and Local Runtime

## Architecture

```text
agent
  definition + registry + policy + invocation contracts
  DefinitionStore / ModelProvider / ToolRegistry / MemoryStore /
  KnowledgeStore / Sandbox traits
                 ↑
  deterministic local adapters (first) → provider/storage/sandbox adapters (later)
                 ↑
  agent::mcp: control-plane validation, registry, bounded invoke
```

The core owns data and policy. Each adapter implements one core-owned trait. The MCP crate receives those traits through dependency injection and never selects a provider or constructs a storage/sandbox adapter itself.

## Definition and registry

`AgentDefinitionV1` contains a schema version, `AgentId`, name, description, model policy, instructions, skill and steering references, allowed tool IDs, policy records, and `ExecutionLimits`. Policies are typed, closed data structures; untyped provider objects and arbitrary context maps are forbidden.

`AgentRegistry` is a pure merge and lookup service over immutable built-ins plus a `DefinitionStore` port. Built-ins are inserted first. User definitions may add new IDs but cannot shadow, update, or delete built-ins. Listing returns a reduced discovery view; getting returns the complete validated definition.

## Local invocation

`LocalAgentRuntime` receives an `AgentRegistry`, a `ModelProvider`, and capability ports. On invocation it:

1. resolves a validated definition;
2. resolves only declared tool IDs and allowed policies;
3. computes `capability_scope_digest` from canonical definition and resolved capability identifiers;
4. constructs one provider-neutral request;
5. processes provider-requested tool calls through the allowed registry only; and
6. emits ordered, normalized events followed by a terminal result.

The initial provider adapter is deterministic and static. It proves policy enforcement and event/result contracts without sending network requests or selecting a model SDK.

## Ports

- `DefinitionStore`: load/list/save user definitions; built-ins are not persisted through this port.
- `ModelProvider`: execute one bounded provider-neutral request and return normalized model/tool-call output.
- `ToolRegistry`: resolve and invoke only registered typed tools by ID.
- `MemoryStore`: scoped recall and write requests.
- `KnowledgeStore`: scoped search requests.
- `Sandbox`: typed execute request; the initial adapter always denies.

Each port has typed request and response values. Core contracts do not expose strings that are interpreted as shell commands, source code, provider configuration, or arbitrary filesystem paths.

## MCP operations

The MCP adapter exposes validation, discovery, registration, and one local invocation. Registration writes only through an injected `DefinitionStore`; invocation returns normalized events/result and stable public errors. It does not expose provider credentials/configuration, arbitrary tool loading, direct sandbox setup, graph execution, or background work.

## Validation strategy

Core tests assert validation and merge semantics. Runtime tests use static/in-memory adapters to assert tool allowlists, scope digest stability, denied sandbox behavior, and normalized events. MCP contract tests assert the five tool names, input schema, safe error mapping, and that it depends only on core contracts plus injected adapters.
