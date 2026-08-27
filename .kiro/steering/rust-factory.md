---
inclusion: always
---
# Rust Factory

Rust Factory is a language- and domain-agnostic foundation for reliable Rust services, libraries, and tools.

## Working conventions

- Design stable, small public interfaces; isolate adapters at system boundaries.
- Prefer idiomatic Rust and established ecosystem crates over custom infrastructure.
- Keep configuration, behavior, and integrations explicit and composable.
- Use Cargo as the source of truth for packages, features, and dependencies.
- Validate changes with the narrowest relevant Cargo command before declaring completion.
