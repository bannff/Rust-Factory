# Requirements: Workflow

## Purpose

Provide the durable, domain-agnostic lifecycle around one bounded Agent invocation. Workflow owns run identity, tenant-scoped persistence, state transitions, evidence, cancellation, and terminal reasons; Agent remains attempt-local and Evaluation remains independent.

## First vertical slice

1. The slice SHALL provide `workflow-core`, `workflow-memory`, and `workflow-mcp` bricks.
2. It SHALL support one immutable, versioned single-step workflow definition whose only step is a named Agent invocation.
3. `workflow-core` SHALL own typed workflow definitions, request context, runs, attempts, append-only events, statuses, terminal reasons, budgets, errors, and core-owned ports.
4. A `WorkflowStore` port SHALL atomically create-or-return a run by `(tenant_id, workflow_id, workflow_version, run_key, input_digest)` and perform compare-and-set updates by revision.
5. `workflow_start` SHALL require a tenant/principal context and nonempty idempotency `run_key`. A conflicting reuse of a key SHALL fail without running the Agent.
6. An injected `AgentInvoker` SHALL execute one resolved Agent attempt. Workflow SHALL persist the invocation's capability-scope digest, normalized events, result/error, and terminal reason as evidence.
7. Runs SHALL transition only through `pending → running → succeeded|failed|cancelled`. Terminal transitions are immutable; cancellation may win only while active.
8. Get/list/update operations SHALL tenant-scope records. Cross-tenant lookup SHALL be indistinguishable from not-found.
9. The MCP adapter SHALL expose only `workflow_validate`, `workflow_start`, `workflow_get`, `workflow_list`, and `workflow_cancel`.

## Safety and ownership

1. Workflow SHALL enforce Factory hard limits on run key, input, evidence, attempts, and emitted events.
2. Agent execution is at-least-once in later recovery/retry layers; external effects require downstream idempotency keys. The first slice has `max_attempts = 1` and no automatic retry.
3. MCP responses SHALL expose logical IDs, statuses, and stable public errors only—never raw store/invoker errors, provider details, host paths, or credentials.
4. Workflow SHALL not implement agent planning, provider selection, tool execution, scoring, evaluation verdicts, graph/swarm logic, worker queues, or mesh coordination.

## Quality requirements

Test idempotent duplicate start, key conflict, CAS/terminal race behavior, cancellation, tenant isolation, unknown workflow/agent, evidence persistence, and stable public MCP mappings. Use deterministic in-memory store and AgentInvoker adapters first.

## Durable-semantics refinements

1. MCP SHALL derive `RequestContext { tenant_id, principal_id }` from an injected authenticated-session resolver. Caller input SHALL not assert tenant or principal identity.
2. `workflow_validate` SHALL resolve trusted request context and authorize workflow-validation policy before it probes referenced Agent availability. `WorkflowDefinitionCatalog` SHALL resolve immutable `(workflow_id, version)` definitions; `AgentInvoker` SHALL validate the referenced `agent_core::AgentId` before start.
3. `WorkflowStore::transition` SHALL atomically commit the expected revision/status check, next run state, attempt mutation, ordered events/evidence, and terminal reason. A conflict returns a typed result without partial publication.
4. This first in-memory synchronous slice supports cancellation only while the local runner has an active cancellation signal registered for the run. `workflow_cancel` SHALL return `conflict` when that local acknowledgement is unavailable; it does not promise durable, cross-process cancellation. Invocation receives that signal, a deadline, and a stable downstream idempotency key derived from tenant, run, and attempt identity. A late completion SHALL never change a terminal state. Durable cancellation leases, recovery, and cross-process acknowledgement are deferred.
5. The exact start identity is `(tenant_id, workflow_id, workflow_version, run_key, sha256(canonical_input))`. Canonical input is a bounded UTF-8 JSON value with object keys recursively sorted, no duplicate keys, and a 64 KiB ceiling. Run, tenant, principal, request, and correlation identifiers use the existing stable logical-ID grammar.
