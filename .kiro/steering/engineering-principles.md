---
inclusion: always
---
# Engineering Principles

These capture *this repo's* decisions, not general Rust. When a rule below and your instinct disagree, follow the rule or raise it.

1. **Polymorphic and agnostic.** Model variation through traits, generics, enums, and data—not domain-specific branches. Keep core logic independent of transports, storage, and vendors. If you find yourself writing `if domain == ...`, the variation belongs in data or a trait impl instead.
2. **Framework-first.** Prefer standard-library, language, and framework primitives over bespoke abstractions. Introduce custom infrastructure only when an identified gap requires it. Confine each vendor crate to one adapter module.
3. **Data-driven and test-driven.** Drive behavior from typed data and configuration where practical. Define expected behavior with focused, deterministic tests, and use `proptest` for input/parameter invariants (the Hypothesis equivalent). Let the compiler carry the load a linter carried elsewhere: encode rules in types so invalid states cannot compile, rather than checking for them at runtime.
4. **Traits are the in-process seam; MCP is the agent control plane.** Brick-to-brick calls in the same process go through a consumed port **trait**—a direct, compiler-checked handshake with no network, serialization, or failure surface. MCP is only for an external agent driving a capability through a bounded, discoverable tool menu. Never route an in-process call through MCP, and never let caller input establish identity, authorization, or trust.
5. **Validate at the boundary, trust the core.** At every external edge: `serde` (with closed schemas) and `schemars` frame the bytes, fallible newtype constructors reject structurally invalid values, and explicit core validation enforces semantic and cross-field rules. Deserialization success is never domain validation. Once a value is a validated core type, downstream code trusts it—no re-checking.
6. **MCP surfaces are self-describing.** An `mcp` module exposes stable contract tools so an agent can discover and drive a brick with zero out-of-band docs—at minimum a capabilities query and a config/schema description. The generated `schemars` schemas are the source of that description; do not hand-maintain a parallel one. This is the "agents drive everything" seam: if an agent needs a human to read code to use a brick, the surface is incomplete.
7. **Single responsibility, small files.** One module owns one concern; a path predicts its contents (`model`, `validation`, `error`, `port`, `service`, adapter modules). Prefer files under ~200 lines. When a file outgrows one clear job, split it along the concern, not arbitrarily.
