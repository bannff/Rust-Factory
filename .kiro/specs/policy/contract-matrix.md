# Policy Contract Matrix

| Consumer | Capability | Policy decision point | Existing domain guard retained |
|---|---|---|---|
| Agent MCP | agent definition/read/register/invoke | before registry/runtime access | agent catalog, definition scope, tool/capability limits |
| Workflow MCP | validate/start/get/list/cancel | before catalog/store/invoker access | tenant-scoped store, idempotency, lifecycle transitions |
| Evaluation MCP | validate/evaluate/get | before reader/store access | tenant-scoped reader/store, immutable result integrity |

Deny maps to a safe public result. Tenant-scoped resource operations use not-found when revealing existence would be unsafe. Allow does not bypass resource tenancy, Agent scope, Workflow lifecycle, Evaluation evidence validation, or downstream idempotency/effect controls.

| Project MCP | intentionally deferred in V1 | no migration | existing bounded project contracts only |

Agent invoke migration requires `EffectiveCapabilityCeilingV1`; test grant-disallowed definition capabilities are neither placed in the model scope nor sent to adapters. All migrated operations test deny before domain calls.