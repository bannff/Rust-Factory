# Policy Tasks

- [x] 1. Create `policy-core` V1 models, validation, closed capabilities, grants, decisions, canonical bytes/digests, and `PolicyResolver`.
- [x] 2. Create `policy-memory` static deterministic grant resolver with fallible construction, duplicate-key and malformed-grant rejection, bounded static record count, and required tests.
- [x] 3. Add Policy V1 golden decision/grant vectors and focused deterministic contract tests.
- [x] 4. Decide `policy-mcp`: deferred because no caller-relative inspection consumer requires it.
- [x] 5. Specify Workflow and Evaluation trusted-context/Policy compatibility adapters before changing their MCP surfaces.
- [x] 6. Migrate Workflow and Evaluation MCP operation-by-operation with contract tests, verified resolver construction, and bounded ingress.
- [x] 7. Specify and migrate Agent MCP to the same trusted-context/Policy boundary; Agent definition sharing remains a separate migration.
- [x] 8. Run Policy/Workflow/Evaluation QA, security, Rust SME, final architecture gates, and `make check` for their accepted slices.

## Accepted compatibility prerequisites

- [x] Define `EffectiveCapabilityCeilingV1` in a separate Agent compatibility spec before Agent MCP migration.
- [x] Prove denied grants cannot reach the policy-protected Workflow Agent model/tool scope.
- [x] Add Cargo membership/dependency tasks: `policy-core`, `policy-memory`, and adapter-only `policy-core` consumption; `sha2.workspace = true` only for approved digest evidence.

## Deferred scope

`policy-mcp`, durable policy persistence/audit projections, and tenant-private Agent definitions require their own approved specifications. The process-local Policy memory adapter does not claim durable authorization evidence.

## Evidence

`policy-core` and `policy-memory` have deterministic canonical decision/grant vectors and static-grant validation tests. Workflow, Evaluation, and Agent MCP Policy compatibility have independent accepted specs, including verified request-bound decisions, deny-before-domain regressions, bounded stdio ingress, required specialist approvals, and `make check` evidence.
