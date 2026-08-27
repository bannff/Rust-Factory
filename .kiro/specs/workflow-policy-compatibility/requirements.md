# Workflow Policy Compatibility

Migrate Workflow MCP to injected trusted context and Policy decisions without changing Workflow core lifecycle semantics.

1. `workflow-mcp` SHALL receive a host-owned trusted-context source and `PolicyResolver` through a compatibility adapter; no caller MCP field establishes identity.
2. After bounded request deserialization and before any catalog/store/invoker access, every operation authorizes its exact `CapabilityV1`.
3. Map: validate→WorkflowValidate; start→WorkflowStart; get→WorkflowGet; list→WorkflowList; cancel→WorkflowCancel.
4. Resolver failure or deny causes zero catalog/store/invoker calls. Validate deny maps to `not_found`; tenant-resource deny maps to `not_found` to prevent enumeration.
5. Policy context converts losslessly to existing `workflow::RequestContext`; Workflow core and its public lifecycle API remain unchanged.
6. V1 introduces no policy persistence, policy MCP, async runtime, or change to local-only cancellation guarantees.

## Attempt-bound grant propagation

For `WorkflowStart`, `WorkflowPolicyContextResolver::authorize(WorkflowStart)` returns trusted `RequestContext`, an allow decision, and request-bound decision digest. The adapter converts allow `GrantV1` losslessly to `agent::EffectiveCapabilityCeilingV1`; `workflow::AgentInvocationRequest` gains `effective_capability_ceiling` and `policy_decision_digest`. Workflow binds both to the created attempt and persists the policy decision digest with capability-scope digest as evidence. The Agent invoker MUST call `invoke_with_ceiling`; a denied grant capability must be absent from model scope and adapters.

`WorkflowPolicyContextResolver::resolve_and_authorize(capability)` is the only MCP pre-domain method. It resolves host context once and authorizes one exact capability. Bounded syntactic input validation occurs before it. For all five operations: source failure → `operation_failed`; policy deny → `not_found`; neither may access catalog/store/invoker. Allow retains current domain behavior and tenant non-disclosure.

## Contract compatibility

Workflow lifecycle semantics and public `WorkflowRunner::{start,get,list,cancel}` behavior remain compatible. The V1 migration intentionally extends only transport-neutral invocation/attempt evidence: `AgentInvocationRequest` and `Attempt` gain immutable `EffectiveCapabilityCeilingV1` and validated 64-hex `policy_decision_digest`. Existing `AgentInvoker` implementers migrate through a compatibility adapter that consumes these values; implementation without the adapter is not considered Policy-protected.

`PolicyResolver::authorize` is decision-only and has no resolver failure channel in V1. `operation_failed` applies only to host trusted-context source or Policy-context conversion failure; Policy deny maps to `not_found`.