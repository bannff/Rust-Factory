# Design: Evaluation Policy Compatibility

```text
already-deserialized MCP parameter DTO
                 ↓ serialized DTO-size framing
host session source → TrustedContextV1
                 ↓ exact Policy capability + verified Allow digest
          semantic validation
                 ↓
       EvaluationService / ports
        ├─ WorkflowEvidenceReader (injected; production bridge is #16)
        ├─ EvaluationExecutor
        └─ EvaluationStore (immutable create-or-match)

composition transport surrounds this adapter:
full envelope bound → deserialization → dispatch; runtime/binding/shutdown
```

`EvaluationPolicyContextResolver<T, P>` belongs to `evaluation::mcp`. It owns injected `TrustedContextSource` and `PolicyResolver` values, resolves trusted context once per request, authorizes one closed Evaluation capability, canonicalizes the effective grant, recomputes the request-bound Allow digest, and returns only the trusted tenant ID needed by Evaluation's tenant-first ports. Its context and decision values remain private implementation details.

## Handler order

Every handler uses the same security order:

1. reserialize the typed parameter DTO and reject it when it exceeds 65,536 bytes;
2. resolve host-derived context and authorize exactly one capability;
3. validate domain semantics only after Allow;
4. call the reader/store only after successful semantic validation;
5. apply result and escaped-output ceilings before returning a safe public projection.

`evaluation_validate`, `evaluation_evaluate_run`, and `evaluation_get_result` map to `EvaluationValidate`, `EvaluationEvaluate`, and `EvaluationGet`. Source failure, deny, and tampered Allow evidence stop before domain effects. Post-Policy semantic failure also stops before domain effects. Keeping semantic validation after authorization avoids an oracle in which a denied caller can distinguish valid from invalid Evaluation identifiers or definitions.

The serialized DTO check is intentionally narrow. `Parameters<T>` already exists when the handler runs, so this check cannot bound the JSON-RPC envelope before buffering or deserialization. Envelope framing is a transport responsibility.

## Core and adapter isolation

Policy changes only the MCP boundary. Evaluation does not use a grant as a runtime capability ceiling because evaluation executors expose no such effect. Request-specific authorization data is excluded from canonical result bytes and hashes.

The injected `EvaluationService` selects an object-safe executor. Both `local::DeterministicCriteriaEvaluator` and `serdes_ai_evals::SerdesAiEvalsExecutor` remain Policy-neutral; `memory::InMemoryEvaluationStore` remains bounded process-local result storage only. The MCP adapter receives no Workflow mutator or Agent invoker.

A production `WorkflowEvidenceReader` is still absent. Issue [#16] owns the acyclic composition bridge and runnable proof; neither Policy compatibility nor `evaluation::memory` supplies it.

## Transport boundary

Evaluation owns no stdio codec, Tokio startup, server loop, or shutdown. A composition transport must enforce the complete MCP/JSON-RPC envelope ceiling before buffering/deserialization, then dispatch a typed DTO to Evaluation. The composition root also derives the host session context and injects Policy and Evaluation adapters. This keeps process lifecycle and transport guarantees outside the capability library.

## Test strategy and evidence limits

Implemented contract tests record source, Policy, reader, and store calls to verify exact capabilities and zero domain effects for source failure, deny, tampered Allow evidence, and post-Policy semantic failure. They also cover the exact three closed schemas, identity/Policy/backend field exclusion, safe projections, injected executor use, and raw/escaped egress ceilings.

These tests do not prove full-envelope ingress bounds, stdio behavior, Tokio lifecycle, a production evidence bridge, or runnable composition. Those require the composition transport and [#16]. Focused Evaluation tests and `make check` passed during this documentation update; specialist approval remains separate acceptance evidence.

[#16]: https://github.com/bannff/Rust-Factory/issues/16
