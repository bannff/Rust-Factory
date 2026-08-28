# Design

```text
host session adapter → TrustedContextV1
                         ↓
              PolicyResolver::authorize(capability)
                         ↓ verified Allow
 evaluation::mcp compatibility adapter → tenant ID → evaluation ports
                                                ├─ WorkflowEvidenceReader (read-only)
                                                └─ EvaluationStore (immutable create-or-match)
```

`EvaluationPolicyContextResolver<T, P>` belongs to `evaluation::mcp`. It owns `TrustedContextSource` and `PolicyResolver`, resolves trusted context exactly once, authorizes one closed Evaluation capability, canonicalizes the effective grant, recomputes the request-bound Allow digest, and returns only the trusted tenant ID needed by Evaluation’s existing tenant-first core ports. Its resolved context and decision values are private implementation details; `EvaluationMcp::new` accepts the verified resolver directly, preventing a caller from minting an authorized tenant context.

Handlers first run existing bounded request serialization and semantic validation. `evaluation_validate` converts and validates `EvaluationDefinitionV1`; `evaluation_evaluate_run` validates its whole bounded request and definition; `evaluation_get_result` validates its whole bounded logical key. Only then may they call the resolver. Source, conversion, canonicalization, or digest verification failure maps to `operation_failed`; deny maps to `not_found`; none may reach the reader/store. An allowed handler follows the existing core path unchanged.

Evaluation does not use a Policy grant as a capability ceiling: it performs no runtime effect. Its canonical result hash excludes request-specific authorization data. This migration therefore changes only the MCP adapter boundary, not Evaluation core, immutable records, or `evaluation::memory`.

## Bounded stdio transport

The adapter owns a private `BoundedStdioTransport` built from rmcp’s bounded JSON-RPC codec. It frames stdin incrementally before deserializing JSON-RPC/parameters, with a 64 KiB inbound frame maximum and no response to an oversize frame; it closes the connection because no trustworthy request ID exists. Valid partial input survives cancelled receives. This mirrors Workflow’s accepted adapter-local protection but is deliberately not extracted as a shared abstraction yet.

## Test strategy

Recording source, Policy, reader, and store adapters prove each operation’s exact capability and zero domain calls for invalid input, source failure, deny, and tampered Allow evidence. Composition tests prove allow preserves tenant isolation, create-or-match semantics, read-only evidence behavior, and safe projections. Transport tests use Tokio duplex for in-limit LF/CRLF and fragmented frames, oversized pre-deserialization termination, and no successor-frame processing after a rejected oversized frame.
