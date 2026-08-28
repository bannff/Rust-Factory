# Policy Contract Matrix

| Consumer | Capability | Policy decision point | Existing domain guard retained |
|---|---|---|---|
| Agent MCP | agent definition/read/register/invoke | before registry/runtime access | agent catalog, definition scope, tool/capability limits |
| Workflow MCP | validate/start/get/list/cancel | before catalog/store/invoker access | tenant-scoped store, idempotency, lifecycle transitions |
| Evaluation MCP | validate/evaluate/get | before reader/store access | tenant-scoped reader/store, immutable result integrity |
| Memory MCP | remember/recall/search/forget/status | before store access, and `memory_enabled` checked after digest verification | tenant/namespace partitioning, capacity ceilings, per-record and framed output bounds |
| Observability MCP | telemetry query/status | before reader/status access | trusted tenant isolation, local/global capacity ceilings, metadata-only bounded projection |

Deny maps to a safe public result. Tenant-scoped resource operations use not-found when revealing existence would be unsafe.

**Unreconciled divergence.** `memory` deliberately departs from the not-found rule and emits a distinct `unauthorized`, on the grounds that a tool's existence is already public through `tools/list` so not-found buys no secrecy, while costing an autonomous caller the ability to stop retrying a capability it will never hold. `observability` emits `operation_failed`, which is worse than either — a permanent refusal indistinguishable from a transient fault. Four surfaces, three contracts. A new surface SHALL NOT simply follow this paragraph until the divergence is reconciled; see `.kiro/specs/memory/requirements.md` section 10.5. Reconciliation is a breaking wire-behaviour change to shipped surfaces and requires its own gate. Allow does not bypass resource tenancy, Agent scope, Workflow lifecycle, Evaluation evidence validation, or downstream idempotency/effect controls.

| Project MCP | intentionally deferred in V1 | no migration | existing bounded project contracts only |

Agent invoke migration requires `EffectiveCapabilityCeilingV1`; test grant-disallowed definition capabilities are neither placed in the model scope nor sent to adapters. All migrated operations test deny before domain calls.