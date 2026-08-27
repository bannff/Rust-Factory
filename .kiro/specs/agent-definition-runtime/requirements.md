# Requirements: Agent Definition and Local Runtime

## Purpose

Prove that Rust Factory can define, register, inspect, and locally invoke a bounded autonomous agent through MCP without coupling the core to an LLM provider, storage system, sandbox implementation, or workflow engine.

## Functional requirements

1. The slice SHALL provide an `agent-core` brick that owns versioned agent definitions, validation, registry rules, capability policies, local invocation contracts, and typed errors.
2. An agent definition SHALL be data: ID, display metadata, model policy/reference, instructions, skills, steering references, allowed tool IDs, memory/knowledge/sandbox/communication policies, and positive execution limits.
3. Agent IDs SHALL use the stable lowercase `[a-z0-9][a-z0-9_-]{0,127}` form. Definitions SHALL reject unknown fields, invalid references, empty required fields, and invalid limits.
4. The registry SHALL merge immutable built-ins before user definitions. A built-in ID wins any collision; user create, update, or delete operations SHALL reject built-in IDs.
5. The core SHALL own small, injected ports for `DefinitionStore`, `ModelProvider`, `ToolRegistry`, `MemoryStore`, `KnowledgeStore`, and `Sandbox`.
6. A local invocation SHALL resolve one validated definition, compute a stable capability-scope digest, allow only the definition's named tools and policies, and return normalized typed events plus a terminal result.
7. Tool, memory, knowledge, and sandbox requests SHALL be typed and policy-scoped. Model output is untrusted and SHALL never expand the resolved capability scope.
8. An MCP adapter SHALL expose bounded `agent_definition_validate`, `agent_definition_get`, `agent_definition_list`, `agent_definition_register`, and `agent_runtime_invoke` operations.

## Safety and ownership requirements

1. The runtime SHALL be attempt-local. Workflow—not this brick—owns durable attempts, cross-attempt budgets, retries, recovery, cancellation, and terminal reasons.
2. The first sandbox adapter SHALL deny execution by default. Any future side-effecting adapter requires explicit policy, identity, authorization, timeouts, bounded output, and audit evidence.
3. MCP responses SHALL expose stable public codes and root-relative or logical identifiers only; they SHALL not disclose provider credentials, raw adapter errors, host paths, or sandbox internals.
4. The core SHALL contain no provider SDK, database, vector-store, MCP, filesystem, network, or framework types.

## Quality requirements

1. Test definition validation, deterministic registry merge/collision behavior, capability-scope digest stability, unknown/disallowed tool rejection, normalized invocation results, and deny-by-default sandbox behavior.
2. Use deterministic in-memory or static adapters for the initial local-runtime tests.
3. Preserve the workspace quality gate: formatting, Clippy with warnings denied, and all workspace tests.

## Explicit non-goals

- Provider selection or integration; streaming model protocols; prompt/tool dynamic loading; arbitrary code execution.
- Graphs, swarms, A2A, durable workflow enrollment, evaluation, mesh networking, CRDT replication, or edge deployment.
- Persistent production storage, vector databases, or sandbox provisioning.
