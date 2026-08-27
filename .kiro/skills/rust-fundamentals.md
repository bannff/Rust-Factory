---
name: rust-fundamentals
description: Compact Rust development guidance for Rust Factory agents.
inclusion: auto
---
# Rust Fundamentals

- Start with the standard library; add a crate only for a clear, maintained capability.
- Use traits, generics, and enums to model variation. Keep concrete implementations behind small interfaces.
- Prefer ownership and lifetimes that make invalid states unrepresentable. Avoid `unsafe` unless documented, isolated, and justified.
- Return typed errors (`thiserror` for libraries, `anyhow` at application boundaries) and preserve error context.
- Keep modules cohesive, public APIs small, and dependencies explicit in `Cargo.toml`.
- Format with `cargo fmt`; lint with `cargo clippy -- -D warnings`; test with `cargo test`.
