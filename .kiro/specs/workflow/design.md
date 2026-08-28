# Design: Workflow

## Architecture

```text
workflow
  WorkflowDefinition / Run / Attempt / Event / RequestContext
  WorkflowStore + AgentInvoker traits
                   ↑
workflow::memory   deterministic in-memory store and static agent invoker
                   ↑
workflow::mcp      validate / start / get / list / cancel
```

The core owns durable lifecycle semantics but no storage, agent, provider, or MCP framework types. `workflow::memory` implements the first deterministic store. `workflow::mcp` receives a store and invoker through dependency injection. Dependencies flow inward.

## Model

`WorkflowDefinitionV1` contains logical ID, immutable version, one `AgentStep { agent_id }`, and `WorkflowBudget { max_attempts: 1, max_input_bytes, max_evidence_bytes }`.

`RequestContext` freezes tenant ID, principal ID, request ID, and correlation ID at run creation. `Run` holds its workflow identity/version, run key, canonical input digest, status, revision, terminal reason, and bounded evidence pointer/count. `Attempt` holds attempt ID, agent ID, capability-scope digest, status, and normalized result/error. `WorkflowEvent` is append-only and ordered within a run.

## Start and transition

1. Validate definition, context, run key, input size, and the resolved Agent ID.
2. Canonicalize the start identity and ask `WorkflowStore` to create-or-return the tenant-scoped run.
3. If the key matches an existing run, return its summary without another Agent invocation. If it conflicts, return `run_key_conflict`.
4. CAS `pending → running`, append `started`, invoke the injected Agent once, append its normalized evidence, then CAS to `succeeded` or `failed`.
5. `cancel` CASes an active run to `cancelled`; terminal status cannot change.

## Ports

- `WorkflowStore`: create-or-return, get/list by tenant, CAS update, append/read events, and persist/read attempts.
- `AgentInvoker`: invokes one logical Agent step with frozen request context, input, attempt ID, idempotency key, cancellation signal, and deadline. It streams bounded evidence chunks into a core-owned sink and returns only the scope digest, so provider evidence is bounded before core storage.
- `Clock` is deferred; the deterministic first implementation receives explicit timestamps/sequence values from the service.

## MCP

The five operations use typed schemas and safe response projections. `workflow_start` is the only side-effecting operation and needs a run key plus tenant/principal fields. `workflow_get` and `workflow_list` return tenant-safe summaries. `workflow_cancel` returns the terminal projection. Resume, event injection, leases, retries, workers, and persistent backends are excluded.

## Evaluation boundary

Evaluation reads terminal Workflow evidence through a separate read port and emits immutable verdict records. It never reruns an Agent, changes Workflow status, or decides a transition. A later workflow projection may link an evaluation pointer, but ownership remains separate.

## Durable transition contract

`WorkflowDefinitionCatalog` resolves immutable definitions; `AgentInvoker` validates the referenced `agent::AgentId`. An injected authenticated `RequestContextResolver` derives tenant/principal context at MCP ingress.

`WorkflowStore::transition(expected_revision, expected_status, mutation)` is the only mutation primitive. One successful transition atomically publishes the next snapshot, continuous attempt mutation, strictly appended ordered events/evidence, and a status-compatible terminal reason; invalid mutations publish nothing. Cancellation supplies a signal to the invoker and transitions an active locally registered run to terminal `cancelled`; late completion cannot publish a competing success/failure transition.

## Cancellation scope

The first in-memory synchronous adapter provides only process-local active-signal cancellation. A cancel request without a locally registered active signal returns `conflict`; no durable cross-process cancellation, lease, recovery, or acknowledgement guarantee is claimed. Those semantics are explicitly deferred to a later persistent workflow adapter.

The start identity uses tenant, workflow ID/version, run key, and SHA-256 of canonical bounded JSON input. The Agent attempt receives a stable tenant/run/attempt-scoped downstream idempotency key.
