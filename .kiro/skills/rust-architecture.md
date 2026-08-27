---
name: rust-architecture
description: Architecture guidance for modular, domain-agnostic Rust systems.
inclusion: auto
---
# Rust Architecture

- Put domain contracts and business rules in a transport-independent core crate.
- Express external seams as small traits owned by the core; adapters implement them and depend on the core, never the reverse.
- Use typed identifiers, enums, and constructors to represent valid state and make transitions explicit.
- Keep public APIs stable and minimal. Prefer composition over inheritance-like hierarchies.
- Keep vendor, database, network, and framework types at adapter boundaries.
- Model domain variation as data and implementations, not `if domain == ...` branches.
