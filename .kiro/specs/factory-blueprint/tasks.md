# Roadmap

- [x] Establish Project and Agent vertical slices.
- [x] Establish Workflow lifecycle contract with a process-local in-memory adapter.
- [x] Evaluation initial slice accepted: immutable evidence assessment, security/architecture approvals, and quality gate recorded.
- [x] Migrate Evaluation MCP to the approved Policy compatibility adapter with bounded pre-deserialization stdio ingress.
- [x] Specify Policy trusted-context/grant contract.
- [x] Define declarative adapter-portfolio, experiment-evidence, and deferred-promotion doctrine without adding a runtime registry or core framework dependency.
- [x] Extract a shared bounded `mcp-transport` adapter after its framing contract is Rust-SME-approved and migrate all MCP adapters to it.
- [ ] Prove one thin local MCP server composition root before adding remaining brick servers or any edge binary.
- [ ] Specify deployment and Policy behavior across process boundaries before any remote brick-client/cloud topology claim.
- [ ] Specify an edge-local linked composition binary only after the local server/root contract is proven; mesh remains deferred.
- [x] Implement the issue #34 async LLM Gateway migration: Agent now depends inward on `llm_gateway::LlmProvider`, and Workflow propagates one borrowed `InvocationControl` with composition-injected deadline wake mechanics. The earlier synchronous Agent-owned `ModelProvider`/`StaticModelProvider` contract is historical and superseded; no future work should use it.
- [ ] Specify an async Sandbox port only after a concrete non-blocking adapter demonstrates the need.
- [ ] Specify an adapter selection/planning implementation only after a demonstrated Project consumer and Rust SME design gate.
- [ ] Extract Tool, Memory, Knowledge, or Sandbox only after a second consumer and one-way migration spec are approved.
- [ ] Add persistent Workflow adapter with specified leases/recovery/cross-process cancellation.
- [ ] Add Evaluation promotion/projection only after Policy and durable Workflow exist.
- [ ] Design Edge/Mesh only after local contracts are stable.

Each item requires its own spec, Rust SME approval, QA/security gates, and `make check`.