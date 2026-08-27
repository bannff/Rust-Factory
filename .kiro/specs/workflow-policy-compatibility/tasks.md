# Tasks

- [x] Add adapter-facing `policy` dependency to `workflow::mcp` only.
- [x] Define host trusted-context source and Policy compatibility resolver/conversion.
- [x] Authorize each Workflow MCP operation before domain access.
- [x] Add recording-port deny/failure zero-call tests for validate/start/get/list/cancel.
- [x] Add allow-path and tenant non-disclosure regression tests.
- [x] Run Rust SME, QA, security, architecture, and `make check`.

- [x] Add attempt-bound `effective_capability_ceiling` and `policy_decision_digest` to workflow invocation/attempt evidence contracts.
- [x] Add Policy-aware AgentInvoker compatibility adapter in workflow::mcp; no mutable request-ID state.
- [x] Add `resolve_and_authorize(capability)` host adapter and exact deny/source failure mappings.
- [x] Prove start grant narrowing reaches neither model scope nor agent adapters for denied capabilities.

- [x] Confirm lifecycle API compatibility while intentionally extending invocation/attempt evidence contracts.
- [x] Add immutable ceiling/digest transition-invariant tests for success, failure, and cancellation.

## Acceptance evidence

- QA approved the final protected MCP path and adversarial regressions.
- Security approved the verified policy boundary and the 64 KiB pre-deserialization stdio frame cap.
- Architecture approved the implementation subject to this checklist reconciliation.
- `make check` passed after the final bounded-transport change.
