# Design: Agent Definition and Local Runtime

## Architecture

```text
agent
  definition + registry + policy + invocation contracts
  DefinitionStore / ToolRegistry / MemoryStore / KnowledgeStore / Sandbox
                 │ async generation through llm_gateway::LlmProvider
                 ↑
  deterministic local adapters (first) → provider/storage/sandbox adapters (later)
                 ↑
  agent::mcp: control-plane validation, registry, bounded invoke
```

The core owns Agent data, scope, trusted `InvocationContextV1`, and policy. LLM Gateway owns the provider-neutral generation port and bounded request/response/evidence types. Each Agent adapter implements one Agent-owned core trait; `LocalAgentRuntime` receives `llm_gateway::LlmProvider` by inward dependency. The MCP module receives those traits, the provider, trusted context, and composition-owned invocation-control factory through dependency injection and never selects a provider or constructs storage/sandbox adapters itself.

> **Issue #34 supersession:** the original delivered design used Agent-owned synchronous `ModelProvider`, `ModelRequest`, and `StaticModelProvider` contracts. Those names below would be historical only; current and future implementation uses `llm_gateway::LlmProvider`, borrowed async `InvocationControl`, explicit Agent `InvocationContextV1`, and `llm_gateway::r#static::StaticProvider`.

## Definition and registry

`AgentDefinitionV1` contains a schema version, `AgentId`, name, description, model policy, instructions, skill and steering references, allowed tool IDs, policy records, and `ExecutionLimits`. Policies are typed, closed data structures; untyped provider objects and arbitrary context maps are forbidden.

`AgentRegistry` is a pure merge and lookup service over immutable built-ins plus a `DefinitionStore` port. Built-ins are inserted first. User definitions may add new IDs but cannot shadow, update, or delete built-ins. Listing returns a reduced discovery view; getting returns the complete validated definition.

## Local invocation

`LocalAgentRuntime` receives an `AgentRegistry`, an `llm_gateway::LlmProvider`, and Agent-owned capability ports. Each invocation also receives a trusted `InvocationContextV1` plus one borrowed async `llm_gateway::InvocationControl`. On invocation it:

1. resolves a validated definition;
2. intersects the effective capability ceiling and resolves only allowed policies/tool IDs;
3. computes `capability_scope_digest` from canonical definition and resolved capability identifiers, excluding invocation identity;
4. constructs one bounded `llm_gateway::GenerateRequest`;
5. awaits the provider, plans and scope-checks every returned call, and passes the unchanged invocation context only to Agent-owned effect ports; and
6. emits ordered, normalized events followed by a terminal result with bounded `InvocationModelEvidence`.

Deterministic tests use `llm_gateway::r#static::StaticProvider`. Provider clients, stable key creation, cancellation/deadline wake mechanics, runtime/timer ownership, credentials, egress policy, and lifecycle remain composition concerns.

## Ports

- `llm_gateway::LlmProvider`: asynchronously execute one bounded provider-neutral generation request under borrowed invocation control and return normalized output/evidence.
- `DefinitionStore`: load/list/save user definitions; built-ins are not persisted through this port.
- `ToolRegistry`: resolve and invoke only registered typed tools by ID.
- `MemoryStore`: scoped recall and write requests with explicit invocation context.
- `KnowledgeStore`: scoped search requests with explicit invocation context.
- `Sandbox`: typed execute request with explicit invocation context; the initial adapter always denies.

Each port has typed request and response values. Core contracts do not expose strings that are interpreted as shell commands, source code, provider configuration, or arbitrary filesystem paths.

## MCP operations

The MCP adapter exposes validation, discovery, registration, and one local invocation. Registration writes only through an injected `DefinitionStore`; invocation returns normalized events/result and stable public errors. It does not expose provider credentials/configuration, arbitrary tool loading, direct sandbox setup, graph execution, or background work.

## Validation strategy

Core tests assert validation and merge semantics. Runtime tests use static/in-memory adapters to assert tool allowlists, scope digest stability, denied sandbox behavior, and normalized events. MCP contract tests assert the five tool names, input schema, safe error mapping, and that it depends only on core contracts plus injected adapters.
