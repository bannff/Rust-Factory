# Evaluation Policy Compatibility

Evaluation MCP uses host-derived trusted context and exact Policy V1 decisions while preserving Evaluation's immutable, read-only core semantics.

1. Only the `evaluation::mcp` adapter has a local dependency on `policy`; the framework-neutral core, `local`, `memory`, `serdes_ai_evals`, and `settings` remain Policy-free.
2. `evaluation::mcp` SHALL receive a host-owned `TrustedContextV1` source plus `PolicyResolver` through `EvaluationPolicyContextResolver<T, P>`. No caller MCP field establishes tenant, principal, request, or correlation identity.
3. Each handler SHALL first enforce the serialized parameter-DTO size ceiling. This is framing validation only and SHALL NOT perform semantic domain validation.
4. After DTO framing, but before semantic validation or reader/store access, the adapter SHALL resolve trusted context and authorize the operation's exact closed capability: `evaluation_validate` → `EvaluationValidate`; `evaluation_evaluate_run` → `EvaluationEvaluate`; `evaluation_get_result` → `EvaluationGet`.
5. The resolver SHALL canonicalize an Allow grant and recompute its request-bound decision digest. Host-source failure, trusted-context failure, canonicalization failure, or a tampered Allow digest maps to `operation_failed`; deny maps to `not_found`. Each failure/deny path makes zero reader/store calls.
6. Semantic validation SHALL occur only after successful authorization. This ordering prevents denial from revealing whether a definition, run ID, or result key is semantically valid. Semantically invalid requests make zero reader/store calls.
7. An allowed, valid operation preserves Evaluation behavior: tenant-first evidence/result lookup, immutable create-or-match records, stable public projections, exact V1 hashes, and no Workflow mutation.
8. Evaluation SHALL NOT consume `GrantV1` as an execution ceiling, persist a request-specific decision digest, mutate Workflow, invoke an Agent, introduce experiment execution/promotion, expose Policy over MCP, retry work, or add Policy to a core module. A Policy decision authorizes one request and is not semantic Evaluation-result content.
9. The adapter SHALL enforce bounded canonical result, serialized result, and JSON-escaped tool-text egress and project private adapter/framework failures only as safe public errors.

## Transport ownership

The 65,536-byte Evaluation request limit measures the serialized parameter DTO after MCP/JSON-RPC deserialization. It is not a full envelope limit and cannot protect pre-deserialization buffering.

A composition transport SHALL bound the complete MCP/JSON-RPC envelope before buffering/deserialization and own connection behavior for oversized input. The composition root owns stdio or other transport binding, Tokio/runtime startup, trusted-context source construction, Policy composition, concrete adapter injection, cancellation/shutdown, and process lifecycle. Evaluation SHALL NOT provide a private stdio transport or claim those guarantees.

## Compatibility

The three tool names and exact capabilities are unchanged. `EvaluationMcp` accepts an injected `EvaluationService` and verified `EvaluationPolicyContextResolver`; its schemas remain closed and expose no identity, Policy, grant, decision, or backend fields.

The core public contracts, object-safe executor seam, local and `serdes-ai-evals` executors, bounded result store, canonical bytes, and exact V1 hashes remain independent of request-specific authorization. `evaluation::memory` stores results only. Issue [#16] separately owns the production Workflow-to-Evaluation evidence bridge and runnable composition proof.

[#16]: https://github.com/bannff/Rust-Factory/issues/16
