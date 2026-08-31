# Rust Factory

Rust Factory is a domain-agnostic foundation for reliable Rust services, libraries, and tools.

## Agent handbook

The durable canonical agent handbook is the Git Wiki revision pinned in [`AGENTS.md`](AGENTS.md): `000f0cf20a0261a497508c5c5af96fbe4a37e352`, tracked by [#42](https://github.com/bannff/Rust-Factory/issues/42). Clone, detach, and verify that exact revision before relying on handbook guidance. The [Wiki Home](https://github.com/bannff/Rust-Factory/wiki/Home) and [Handbook Governance](https://github.com/bannff/Rust-Factory/wiki/Handbook-Governance) pages are non-authoritative browser navigation unless verified against the pin.

Repository executable enforcement takes precedence over the pinned handbook; the handbook takes precedence over the Kiro compatibility snapshot. The complete precedence is system/platform/user instructions → executable repository enforcement and repository state → pinned Wiki handbook → Kiro snapshot → host defaults and floating Wiki content. If the Wiki is unavailable or verification fails, retain system/platform/user instructions and executable enforcement, then use the Kiro snapshot; do not substitute another Wiki revision. [`AGENTS.md`](AGENTS.md) is the complete canonical bootstrap.

## Workspace

One brick is one crate. Its agent-facing MCP surface and its local adapters are
feature-gated modules inside it, so a capability is one thing to find and compose.
No feature is on by default, so a brick's default build resolves no transport,
schema, error-framework, filesystem, or async-runtime dependency.

| Brick | Features | Capability |
|---|---|---|
| `crates/agent` | `mcp` | Versioned agent definitions, registry, and bounded local runtime contracts. |
| `crates/workflow` | `mcp`, `memory` | Bounded workflow lifecycle for one agent invocation. |
| `crates/evaluation` | `local`, `memory`, `mcp`, `serdes-ai-evals`, `settings` | Framework-backed immutable evaluation over terminal Workflow evidence, with selectable deterministic executors and bounded process-local result storage. Composition-ready; not runnable or project-ready until [#16] supplies the evidence bridge and composition proof. |
| `crates/project` | `mcp`, `fs` | Blueprint validation, generation planning, and root-confined materialization. |
| `crates/policy` | `memory` | Trusted context, closed capabilities, and grant decisions. No MCP surface, by design. |
| `crates/auth` | `biscuit` | **Implemented.** Token-native synchronous authorization with identity derived from direct verified-authority facts and bounded opaque deny behavior. No MCP, Policy migration, revocation, audit, or process-boundary guarantee; composition owns private keys and blocking scheduling. |
| `crates/storage` | `local`, `redb`, `settings` | Bounded authoritative tenant- and namespace-scoped versioned objects, with a volatile local adapter and an `ImmediateCommit` redb adapter. No consumer migration or MCP surface in V1. |
| `crates/memory` | `local`, `agentic`, `settings`, `mcp` | Tenant-scoped agent memory behind one framework-agnostic port. Two selectable backends and a five-tool agent surface; no durable adapter. |
| `crates/knowledge` | `static` | **Implemented.** Bounded synchronous framework-free `KnowledgeIndex`/`KnowledgeService` core with a std-only immutable static adapter; Agent migration is complete. No MCP, settings, ingestion, async, ranking, vector, remote, persistence, or lifecycle surface. Host-derived tenant/principal and a validated globally shared Agent definition select tenant + namespace; caller/model/tool cannot select scope, Policy/Agent admits the principal, the static adapter is not principal-partitioned, and global definition visibility neither globalizes the corpus nor authorizes cross-tenant data. Issue evidence, merge, and delivery remain pending. |
| `crates/observability` | `local`, `opentelemetry`, `settings`, `mcp` | Bounded tenant-scoped operational logs with an evicting process-local reader, metadata-only OpenTelemetry API submission, and Policy-gated inspection; no durable audit/evidence guarantee. |
| `crates/llm-gateway` | `static`, `genai` | **Implemented.** Bounded non-streaming `LlmProvider` core with deterministic static and injected-client genai adapters, Agent/Workflow async migration, trusted invocation context, tenant-isolation coverage, and bounded process-local broadcast cancellation. No MCP or settings surface, retries, streaming, remote-abort acknowledgement, durability, exactly-once, recovery, or process-boundary guarantee. No deployable composition exists. Composition owns configured clients, runtime/timer and deadline wake mechanics, stable invocation keys, cancellation source, credentials, endpoint/egress/TLS/proxy policy, concurrency, task supervision, lifecycle, and shutdown. |
| `crates/sandbox` | — | Status-only scaffold; its provisional port still lives in `agent`. |
| `crates/mcp-transport` | — | Shared bounded MCP stdio transport. Owns no capability. |

A `memory`, `local`, or `fs` module is a deterministic process-local adapter: no
persistence, recovery, lease, or cross-process guarantee. Evaluation keeps these
roles separate: `local::DeterministicCriteriaEvaluator` executes the closed V1
criteria, while `memory::InMemoryEvaluationStore` stores at most 1,024 results
per tenant and 4,096 globally without eviction. `memory` does not adapt Workflow
evidence. The optional `serdes-ai-evals` feature confines the external framework
to `serdes_ai_evals::SerdesAiEvalsExecutor`; both executors satisfy the same V1
verdict, finding-order, digest, and hash contracts through the object-safe
`EvaluationExecutor` seam. Executor futures are caller-polled and cancel only by
drop; the current adapters start no detached work and claim no timeout,
acknowledgement, retry, or recovery behavior.

A vendor module is named for the crate it confines — `agentic` holds
`agentic-memory` and nothing else names it — and a `settings` module holds the
shape of a project's configuration, never its source and never the
selection-to-constructor `match`, which belong to a composition binary. An
adapter is feature-gated even when it adds no dependency, so the rule that a core
module names no adapter keeps applying to it.

`policy` has no MCP surface deliberately. It decides what an agent is permitted
to do, so exposing it to agents would be a privilege-escalation seam whichever
tools were chosen. Storage also deliberately has no MCP surface in V1: exposing
raw object CRUD would bypass each consuming capability's serialization and
domain rules. Agent-driven surfaces are therefore capability-specific, not a
blanket requirement for every operable brick.

Storage retains authoritative versioned objects and never evicts them to recover
capacity. Cache remains a separate, deferred, non-authoritative capability whose
data may be evicted without violating correctness. No Agent or Evaluation
consumer has migrated to Storage. Composition owns configuration source and
adapter selection, plus the trusted redb path and parent directory, path/symlink
and TOCTOU policy, locking/open behavior, backup, lifecycle, and shutdown.

Every package declares `family`, `role`, and `status` in
`[package.metadata.rust-factory]`. The capability roadmap and taxonomy live in
GitHub issue #11 and GitHub Projects; the Vision records architecture rather
than a registry table. The local validator checks workspace membership, package
metadata and placement, status-only package shape, targets, and adapter
isolation; `make check` runs those checks with the feature-matrix quality gate.

## Repository layout

- `crates/` — libraries, one per capability. No binary targets.
- `projects/` — deployable binaries, one per composition root. Not yet created.
- `.kiro/steering` — architecture rules, injected into every agent session.
- `.kiro/specs/brick-standard` — the contract a new or refactored brick follows.
- `.kiro/skills` — shared Rust guidance for all agent roles.

## Build

Requires Rust 1.88+ (edition 2024) and Python 3.11+, which the registry
validator needs for `tomllib`.

```sh
cargo build --workspace   # framework-free cores only
make check                # the full gate, across the feature matrix
```

The workspace produces libraries only. There is nothing to run yet: transport
binding belongs to a `projects/` composition root, and none exists ([#6]).
Evaluation is framework-backed and composition-ready, not runnable or
project-ready: it intentionally has no production `WorkflowEvidenceReader`.
Issue [#16] owns that Workflow-to-Evaluation bridge and the selected-executor
composition proof. Evaluation MCP bounds its serialized parameter DTO only;
the composition transport must bound the full MCP/JSON-RPC envelope before
buffering/deserialization and own stdio or other binding, Tokio/runtime startup,
and shutdown.

[#6]: https://github.com/bannff/Rust-Factory/issues/6
[#16]: https://github.com/bannff/Rust-Factory/issues/16

## Brick standard

New or refactored bricks follow the [Canonical Brick Standard](.kiro/specs/brick-standard/requirements.md). A brick is exactly one crate, named for its capability. Adapters are feature-gated modules inside it — `mcp`, `memory`, `local`, `fs`, a vendor module, `settings` — and a binary under `projects/` owns runtime, transport, configuration, trusted context, Policy composition, concrete adapter injection, and shutdown. Boundary DTOs use `serde` and `schemars`; typed constructors and core `validate_*` rules establish domain validity. No library owns process lifecycle: `serve_stdio` has been removed from every brick.

A shared contract is ordinarily extracted only after at least two demonstrated direct consumers prove a stable narrow need; the extracted crate becomes canonical and consumers depend inward on it. A narrow exception applies when one live provisional port has one demonstrated direct consumer, the tracking issue records an explicit product mandate, the Rust SME approves the replacement before setup, and implementation performs an atomic one-way migration that removes the provisional port without aliases or a compatibility facade. Zero-consumer packages remain prohibited; Storage is the sole approved exception. There is no generic umbrella core.

## Quality gate

Run `make check` before submitting changes. It validates the brick registry,
asserts adapter isolation, formats, lints, and tests — each across the feature
matrix, because with no default features a workspace-wide command would only
exercise the framework-free cores.

`make isolation-check` asserts that each brick's default build resolves none of
the adapter dependencies; the registry validator separately forbids naming one
outside its own module. Neither claims framework-free *artifacts*: Cargo unifies
features per build graph, so a binary composing several bricks with `mcp` enabled
links one framework-carrying build of each.
