---
inclusion: always
---
# Rust Factory Delivery Workflow

Use this workflow for every non-trivial brick, public API change, adapter, concurrency change, or dependency decision. The goal is to apply focused expertise without creating a generic committee or duplicating investigation.

## Standard sequence

1. **Repository context — `context-gatherer`**
   Map existing crates, contracts, call sites, tests, specifications, and steering before design or implementation. Reuse prior context when the question is unchanged.

2. **Rust design gate — `rust-factory-sme`**
   Review the intended crate boundary, trait/adapter ownership, public API, Cargo dependencies, ownership/concurrency model, error taxonomy, resource bounds, and framework choice. Resolve all Blocker and Required findings before implementation.

3. **Implementation — `implementer`**
   Build the smallest vertical slice that proves the brick contract. Apply `rust-fundamentals`, `rust-architecture`, `rust-quality`, and `rust-async` skills. Core crates remain framework-free; adapters receive concrete dependencies through injection.

4. **Adversarial QA — `qa-tester`**
   Add or assess deterministic behavior tests, state-transition/race tests, limits, malformed input, tenant isolation, idempotency, and public-contract cases. Generated fixtures must pass their own quality gate when applicable.

5. **Security gate — `security-reviewer`**
   Review authorization/context derivation, capability boundaries, input/output ceilings, cancellation, filesystem/network confinement, idempotency, evidence handling, secret/path/error leakage, and unsafe side effects. Resolve Blocker and Required findings.

6. **Final API/architecture gate — `meta-architect` and `rust-factory-sme`**
   Confirm the final code remains faithful to the spec and Living Factory Vision, dependency direction is inward, lifecycle ownership is correct, and future adapters have a clean seam.

7. **Quality and documentation**
   Run `make check`. Update the applicable spec tasks, README/API documentation, and living steering only when implementation evidence changes the agreed design. Use `doc-writer` when user-facing or architectural documentation needs review.

## Decision rules

- **Framework-first:** Before adding a dependency, the Rust SME must identify the concrete gap and ensure the chosen crate is contained in an adapter. Pin exact versions in Cargo manifests.
- **No premature async or distribution:** Start synchronous and local. Add Tokio, workers, persistence, mesh, or CRDT adapters only for a demonstrated requirement and through core-owned traits.
- **No false durability claims:** A local/in-memory adapter must describe its actual guarantees. Leases, recovery, cross-process cancellation, retries, and exactly-once effects require their own specified durable adapter.
- **Approval is a gate:** `APPROVE` is required before a brick's spec is marked stable. Tests passing alone are not evidence that architecture or security requirements are met.
- **Scale down for trivial edits:** Documentation-only or mechanical renames may use context → implementation → `make check`; preserve the full gates for behavior, public API, dependencies, and safety changes.
