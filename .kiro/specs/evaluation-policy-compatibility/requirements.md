# Evaluation Policy Compatibility

Migrate Evaluation MCP to host-derived trusted context and Policy V1 decisions while preserving Evaluation’s immutable, read-only core semantics.

1. Only `evaluation-mcp` gains an adapter-facing exact local dependency on `policy-core`; `evaluation-core` and `evaluation-memory` remain Policy-free.
2. `evaluation-mcp` SHALL receive a host-owned `TrustedContextV1` source plus `PolicyResolver` through a verified compatibility resolver. No caller MCP field establishes identity.
3. After bounded syntactic and semantic validation, but before any evidence-reader or result-store access, every operation SHALL authorize its exact closed capability: validate → `EvaluationValidate`; evaluate run → `EvaluationEvaluate`; get result → `EvaluationGet`.
4. The resolver SHALL canonicalize an Allow grant and recompute the request-bound decision digest. Host-source failure, trusted-context conversion failure, canonicalization failure, or a tampered Allow digest maps to `operation_failed`; deny maps to `not_found`. Each failure/deny path makes zero reader/store calls.
5. Invalid or oversized MCP input SHALL make zero trusted-context, Policy, reader, or store calls. Validation checks the definition before authorization; evaluate/get validate their complete bounded input before authorization.
6. An allowed operation preserves current Evaluation behavior: tenant-first evidence/result lookups, immutable create-or-match records, stable public projection, and no Workflow mutation.
7. Evaluation SHALL NOT consume `GrantV1` as an execution ceiling, persist a request-specific decision digest, mutate Workflow, invoke an Agent, introduce experiment execution/promotion, Policy MCP, retries, or a Policy dependency in a core crate. A decision is authorization evidence for one request, not semantic Evaluation-result content.

## Bounded MCP ingress

`evaluation-mcp` SHALL replace direct `rmcp::transport::stdio()` use with an adapter-private bounded newline-delimited JSON-RPC transport. A complete inbound frame, excluding LF and optional CR delimiter, is at most 64 KiB. The transport bounds incrementally before JSON-RPC or `Parameters<T>` deserialization, retains valid partial-frame state across cancelled receives, and terminates the stdio connection without a response on oversized framing input. An oversized frame reaches no trusted-context source, Policy resolver, reader, or store. Valid frames retain rmcp-defined parsing behavior.

The transport is adapter-only. It does not add a framework dependency to Evaluation core/memory or establish a shared Factory runtime abstraction. A common internal MCP utility may be specified only after Evaluation and Workflow prove identical stable requirements.

## Compatibility

The public `EvaluationMcp` constructor intentionally changes from the legacy local `TrustedContextResolver` seam to a verified `EvaluationPolicyContextResolver<T, P>` seam. The legacy context resolver/type is removed rather than retained as an unprotected construction path. `evaluation-core` and `evaluation-memory` public contracts, canonical result bytes/hash, and tenant-first store/evidence APIs remain unchanged.
