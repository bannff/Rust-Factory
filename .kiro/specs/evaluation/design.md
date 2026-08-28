# Design: Evaluation

```text
composition-owned WorkflowEvidenceReader bridge (#16, not implemented)
                              ↓
framework-neutral evaluation core
  model / validation / canonical / error / port / service
  EvaluationExecutor + WorkflowEvidenceReader + EvaluationStore
                ↓                         ↓
        executor adapters          result-store adapter
        local                      memory
        serdes_ai_evals
                ↑
        settings selects a backend in a composition root
                ↑
        mcp: DTO framing → Policy → semantics/effects
                ↑
        composition transport: envelope bounds and lifecycle
```

Evaluation owns deterministic V1 criteria, an Evaluation-owned terminal evidence projection, canonical immutable records, and the orchestration that binds trusted evidence to a result. It does not consume `workflow::Run`, depend on `workflow`, or provide a production `WorkflowEvidenceReader`. Issue [#16] owns that bridge, its tenant/terminal-state tests, and the runnable composition proof.

## Module and feature boundaries

The default feature set is the framework-neutral core:

- `model`, `validation`, `canonical`, and `error` own V1 types, limits, validation, canonical bytes, and public error projection.
- `port` owns the object-safe `EvaluationExecutor`, `WorkflowEvidenceReader`, and `EvaluationStore` seams. `EvaluationFuture` is a boxed standard-library future, so core selects no async runtime.
- `service` validates and binds the request/evidence, invokes the injected executor, validates its assessment, computes all semantic identities, and optionally calls immutable create-or-match storage.

Opt-in features contain adapters:

- `local` exposes `DeterministicCriteriaEvaluator`, the runtime-free reference executor.
- `serdes-ai-evals` exposes the Rust module `serdes_ai_evals` and `SerdesAiEvalsExecutor`. The framework crate is confined to that module. Exact output uses its exact-match scorer; the event predicates use private bounded function scorers. Framework outcomes are reduced to core-owned ordered findings, and framework details never enter result hashes or public errors.
- `memory` exposes `InMemoryEvaluationStore` only. It stores process-local results behind a mutex and ordered map; it is not a Workflow evidence adapter.
- `settings` exposes closed V1 Serde/Schemars configuration for the two executor names and bounded in-memory store. It does not read files or construct adapters.
- `mcp` exposes the unchanged three-tool Policy-gated control plane.

A composition root selects concrete adapters and supplies the missing `WorkflowEvidenceReader`, Policy resolver, trusted-context source, transport, runtime, and shutdown behavior.

## Executor and cancellation model

`EvaluationExecutor::assess` accepts validated `EvaluationDefinitionV1` and `TerminalEvidenceSnapshotV1` references and returns only `EvaluatorAssessmentV1`. The trait is object-safe and supported behind `Arc<dyn EvaluationExecutor>`. Core, not the executor, constructs the logical key, evidence digest, content hash, and immutable record.

`DeterministicCriteriaEvaluator` and `SerdesAiEvalsExecutor` are deterministic, preserve criterion order, require no runtime, perform no external I/O or network access, and start no detached work. The truthful cancellation contract is cancellation by dropping the caller-owned future. There is no acknowledgement, deadline enforcement, cross-process cancellation, retry, or recovery claim; Workflow and the eventual composition own those concerns.

## Immutable storage

`InMemoryEvaluationStore` implements atomic create-or-match within one process. Defaults and maximum configurable capacities are 1,024 results per tenant and 4,096 globally. It checks an existing key before capacity, so an identical match or conflict remains observable at capacity. A new result that exceeds either ceiling is rejected with no insertion or eviction. The adapter explicitly reports no restart durability, cross-process visibility, crash atomicity, or eviction.

`memory` cannot read Workflow evidence. Reintroducing that dependency in Evaluation would invert the capability boundary and risk a Cargo cycle. The acyclic composition-owned bridge remains [#16].

## MCP and Policy ordering

The MCP surface remains exactly:

| Tool | Policy capability | Effect after authorization and semantics |
|---|---|---|
| `evaluation_validate` | `EvaluationValidate` | Validate one V1 definition; no reader/store effect. |
| `evaluation_evaluate_run` | `EvaluationEvaluate` | Read one terminal snapshot, evaluate it, then create-or-match a result. |
| `evaluation_get_result` | `EvaluationGet` | Read one tenant-scoped immutable result. |

Each handler performs this order:

1. serialize the already-deserialized parameter DTO and enforce the adapter's 65,536-byte DTO ceiling;
2. resolve host-derived `TrustedContextV1` and authorize the exact closed capability, including Allow-digest verification;
3. perform semantic definition/key validation;
4. invoke the reader/store only for an allowed, semantically valid operation;
5. enforce canonical-result, serialized-result, and escaped tool-text egress ceilings and return a safe projection.

Semantic validation deliberately follows Policy so denial does not disclose whether a request was semantically valid. The DTO check is framing validation only: it is not a full MCP/JSON-RPC envelope bound because deserialization has already occurred. The composition transport must reject oversized envelopes before buffering/deserialization and owns stdio or other binding, Tokio startup, connection behavior, and shutdown. Evaluation owns none of that lifecycle.

## V1 compatibility

Canonical bytes remain length-prefixed UTF-8 with fixed order, ASCII decimal integers, exact lowercase enums, and explicit list counts. Backend selection and Policy evidence are excluded from semantic bytes. The shared golden cohort remains:

- evidence: `400d023425c9ee77e3eb9ac40032e0871dcc3eaf6980b743f29fccdc025150eb`
- definition: `5c94014a3ba627135274d1cf4c9b54e2c06af1a24e396d8d6dc3c5f6ab90d401`
- result: `03414bc05e2c0b4aae494cc0fe12473da48fa0922f637e3836662839a5bebe72`

Cross-adapter contract tests require byte-semantic parity. New score, rubric, rationale, model-judge, or provenance semantics require a separately specified V2 encoding rather than reinterpretation of V1.

## Readiness boundary

The brick is framework-backed and composition-ready: its core, adapters, settings, bounds, and injected seams exist. It is not runnable or project-ready because no composition root supplies a production Workflow evidence bridge. Issue [#16] is the explicit blocker. Focused all-feature tests and `make check` passed during this documentation update; final meta-architecture approval remains separate acceptance evidence and must not be inferred from those checks.

[#16]: https://github.com/bannff/Rust-Factory/issues/16
