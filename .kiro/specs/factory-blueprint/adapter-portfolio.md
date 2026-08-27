# Adapter Portfolio, Experiments, and Promotion Doctrine

Rust Factory is both a project authoring system and a controlled research environment for Rust services and agentic workflows. A brick exposes a stable, framework-free core contract; projects select concrete adapters appropriate to their requirements; experiments compare adapter choices without redefining core ownership.

## Adapter portfolio

An eventual project-planning schema MAY declare bounded `AdapterDescriptorV1` and `AdapterSelectionV1` records. Each descriptor names a stable logical adapter ID, the brick and typed port it implements, adapter kind, exact Cargo package/version, compatibility contract version, declared limits, and honest guarantee class (`process_local`, `durable`, or a separately specified stronger class).

Selections are declarative planning data only. Planning validates unique selections, known ports, contract compatibility, one-way dependencies from adapter to core, and the prohibition on framework dependencies in core crates. Generated projects compose selected crates with normal Cargo dependencies and explicit constructor injection.

Rust Factory SHALL NOT use runtime plugin loading, dynamic crate discovery, a global service locator, or an implicit universal adapter registry. Cargo resolution and compile-time constructor wiring are the composition authority. A framework is contained in a named adapter crate (`<brick>-<framework>`) or an experiment crate; it never becomes a dependency of the core merely because a project selected it.

## Experiments

An experiment is a bounded, reproducible composition of existing typed ports, adapter selections, inputs, and measurement plan. It may run a candidate orchestration, model/provider, sandbox, storage, UI, or evaluation harness, but it must not create a competing source of Workflow lifecycle state.

Workflow remains the owner of budgets, cancellation, idempotency, terminal state, and attempt evidence. Evaluation remains a read-only assessor of immutable terminal evidence. Experiments consume these contracts and emit immutable references to candidate identity/version, adapter compatibility validation, terminal workflow evidence, evaluation result content hash, and measurement artifacts. Memory and in-memory adapters are process-local evidence helpers only; they do not establish durability, recovery, leases, retries, or cross-process cancellation.

Initial experimentation belongs in `experiments/<name>/` or an explicitly named adapter crate. Promotion to the supported portfolio requires a dedicated specification, a stable core-owned port or demonstrated existing port, at least one concrete consumer, bounded tests, and the standard delivery gates.

## Promotion is deferred

A PASS evaluation is not an adapter promotion. A future promotion projection must be separately specified and policy-authorized. It will verify immutable candidate, compatibility, workflow, and evaluation references; use explicit idempotency/concurrency semantics; and write its own projection without mutating Workflow state or Evaluation records.

Rust Factory currently has no durable Workflow or promotion adapter. It therefore makes no durable recommendation, release, self-modification, or automatic framework-selection claim.
