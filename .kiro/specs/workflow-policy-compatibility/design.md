# Design

```text
host session adapter → TrustedContextV1
                         ↓
              PolicyResolver::authorize(capability)
                         ↓ allow
workflow-mcp compatibility adapter → workflow::RequestContext → runner
```

Create a `WorkflowPolicyContextResolver` adapter in `workflow-mcp` that owns the host context source and `PolicyResolver`. It resolves context once, authorizes the requested closed capability, converts IDs to `workflow::LogicalId`, and returns the existing `RequestContext`. Every MCP handler calls this adapter before its domain path. Denied/error returns a safe Workflow public error; no new dependency enters `workflow`.

Tests use recording catalog/store/invoker and policy source to prove deny/failure is pre-domain for all five operations; allow preserves current behavior and tenant isolation.

## Start propagation

`workflow-mcp` receives a Policy-aware `AgentInvoker` compatibility adapter. It converts only an allow decision’s grant to `EffectiveCapabilityCeilingV1`, copies the policy decision digest into `AgentInvocationRequest`, and passes both through `workflow` untouched. `workflow` records policy decision digest beside the Agent scope digest in `Attempt`. No mutable request-ID lookup is used; grant/decision are values carried by the synchronous attempt request.

Tests use recording host context, policy resolver, catalog/store/invoker, and Agent runtime to prove: exact capability per handler; deny/source failure zero domain calls; start grant denies an Agent capability before model/adapter access; decision/scope evidence persists with the attempt; and allow preserves lifecycle/idempotency/tenant behavior.

The Policy-aware invoker adapter is the sole V1 implementation of the extended `AgentInvoker` contract for policy-protected workflow execution. It rejects missing/invalid decision digest or ceiling before forwarding to Agent. Workflow transition validation treats ceiling and decision digest as immutable attempt identity fields across terminal transitions.