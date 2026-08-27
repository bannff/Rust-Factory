# Roadmap

- [x] Establish Project and Agent vertical slices.
- [x] Establish Workflow lifecycle contract with a process-local in-memory adapter.
- [x] Evaluation initial slice accepted: immutable evidence assessment, security/architecture approvals, and quality gate recorded.
- [x] Migrate Evaluation MCP to the approved Policy compatibility adapter with bounded pre-deserialization stdio ingress.
- [x] Specify Policy trusted-context/grant contract.
- [x] Define declarative adapter-portfolio, experiment-evidence, and deferred-promotion doctrine without adding a runtime registry or core framework dependency.
- [ ] Specify an adapter selection/planning implementation only after a demonstrated Project consumer and Rust SME design gate.
- [ ] Extract Tool, Memory, Knowledge, or Sandbox only after a second consumer and one-way migration spec are approved.
- [ ] Add persistent Workflow adapter with specified leases/recovery/cross-process cancellation.
- [ ] Add Evaluation promotion/projection only after Policy and durable Workflow exist.
- [ ] Design Edge/Mesh only after local contracts are stable.

Each item requires its own spec, Rust SME approval, QA/security gates, and `make check`.