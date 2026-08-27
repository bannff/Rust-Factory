# Tasks: Agent Definition and Local Runtime

- [x] 1. Create `agent` with versioned definition, policy, limit, registry, invocation, event/result, and error contracts.
- [x] 2. Implement explicit validation for IDs, required fields, references, policy shape, and positive execution limits.
- [x] 3. Define `DefinitionStore`, `ModelProvider`, `ToolRegistry`, `MemoryStore`, `KnowledgeStore`, and `Sandbox` traits owned by the core.
- [x] 4. Implement deterministic in-memory registry/store and immutable built-in merge behavior.
- [x] 5. Implement `LocalAgentRuntime` capability resolution, stable scope digest, normalized event/result flow, and tool allowlist enforcement.
- [x] 6. Add static-model, fixed-tool, in-memory-memory/knowledge, and deny-by-default-sandbox test adapters.
- [x] 7. Add focused core/runtime tests for registry collision precedence, invalid definitions, scope stability, rejected unknown/disallowed tools, and denied sandbox access.
- [x] 8. Create `agent::mcp` with injected registry/store/runtime ports.
- [x] 9. Add bounded MCP operations: `agent_definition_validate`, `agent_definition_get`, `agent_definition_list`, `agent_definition_register`, and `agent_runtime_invoke`.
- [x] 10. Contract-test MCP schemas, stable public error mapping, and absence of credential/provider/host-path leakage.
- [x] 11. Run generated and Factory workspace quality gates; review the stable local agent API before proposing workflow/evaluation or mesh/edge adapters.

## Validation matrix

Narrow command: `cargo test -p agent --features mcp`. Required: definition/catalog validation, capability limits, denial paths, scope-digest stability, MCP schema/projection. Conditional: proptest for policy scope; loom after shared-state adapters; fuzz ingress. N/A: canonical golden records (Agent has no immutable record codec).