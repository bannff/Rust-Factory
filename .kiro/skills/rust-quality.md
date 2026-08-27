---
name: rust-quality
description: Quality, verification, and maintenance guidance for Rust Factory agents.
inclusion: auto
---
# Rust Quality

- Add focused, deterministic tests for observable behavior and boundary conditions.
- Use unit tests for domain rules; use integration tests for public crate behavior; use property tests only where invariants benefit from generated input.
- Keep production code `unsafe`-free by default. Any exception needs a documented safety invariant and a narrowly scoped review.
- Before completion run `cargo fmt --check`, `cargo clippy --workspace --all-targets -- -D warnings`, and `cargo test --workspace`.
- Keep errors typed and actionable; do not panic for recoverable input or integration failures.
- Prefer clear code and small modules over abstractions that lack a demonstrated reuse case.
