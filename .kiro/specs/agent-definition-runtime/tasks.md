# Tasks: Agent Definition and Local Runtime

> Tasks 1-11 record the historically delivered initial slice. Issue #34 later superseded its Agent-owned synchronous `ModelProvider`/`ModelRequest`/`StaticModelProvider` implementation with direct `llm_gateway::LlmProvider`, borrowed async `InvocationControl`, explicit Agent `InvocationContextV1`, and `llm_gateway::r#static::StaticProvider`. Preserve the checked history, but use only the current LLM Gateway APIs for future work.

- [x] 1. Create `agent` with versioned definition, policy, limit, registry, invocation, event/result, and error contracts.
- [x] 2. Implement explicit validation for IDs, required fields, references, policy shape, and positive execution limits.
- [x] 3. Historically defined Agent-owned `DefinitionStore`, synchronous `ModelProvider`, `ToolRegistry`, `MemoryStore`, `KnowledgeStore`, and `Sandbox` traits. Issue #34 superseded only the provider portion with `llm_gateway::LlmProvider`; this checkbox remains historical delivery evidence.
- [x] 4. Implement deterministic in-memory registry/store and immutable built-in merge behavior.
- [x] 5. Implement `LocalAgentRuntime` capability resolution, stable scope digest, normalized event/result flow, and tool allowlist enforcement.
- [x] 6. Historically added a static-model test adapter with fixed-tool, in-memory-memory/knowledge, and deny-by-default-sandbox adapters. Issue #34 replaced that provider adapter with `llm_gateway::r#static::StaticProvider`.
- [x] 7. Add focused core/runtime tests for registry collision precedence, invalid definitions, scope stability, rejected unknown/disallowed tools, and denied sandbox access.
- [x] 8. Create `agent::mcp` with injected registry/store/runtime ports.
- [x] 9. Add bounded MCP operations: `agent_definition_validate`, `agent_definition_get`, `agent_definition_list`, `agent_definition_register`, and `agent_runtime_invoke`.
- [x] 10. Contract-test MCP schemas, stable public error mapping, and absence of credential/provider/host-path leakage.
- [x] 11. Run generated and Factory workspace quality gates; review the stable local agent API before proposing workflow/evaluation or mesh/edge adapters.

## Validation matrix

Narrow command: `cargo test -p agent --features mcp`. Required: definition/catalog validation, capability limits, denial paths, scope-digest stability, MCP schema/projection. Conditional: proptest for policy scope; loom after shared-state adapters; fuzz ingress. N/A: canonical golden records (Agent has no immutable record codec).