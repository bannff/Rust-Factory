---
inclusion: always
---
# Engineering Principles

 When a rule below and your instinct disagree, follow the rule or raise it.

1. **Polymorphic and agnostic.** Model variation through traits, generics, enums, and data—not domain-specific branches. Keep core logic independent of transports, storage, and vendors. If you find yourself writing `if domain == ...`, the variation belongs in data or a trait impl instead.
2. **Framework-first.** Prefer standard-library, language, and framework primitives over bespoke abstractions. Introduce custom infrastructure only when an identified gap requires it. Confine each vendor crate to one adapter module.
3. **Data-driven and test-driven.** Drive behavior from typed data and configuration where practical. Define expected behavior with focused, deterministic tests, and use `proptest` for input/parameter invariants (the Hypothesis equivalent). Let the compiler carry the load a linter carried elsewhere: encode rules in types so invalid states cannot compile, rather than checking for them at runtime.
4. **Traits are the in-process seam; MCP is the mandatory agent control plane.** Same-process brick calls use consumed port traits, not MCP. Every mature capability brick owns a transport-agnostic MCP handler, and every project exposes exactly one unified MCP server process containing its statically selected brick handlers plus project meta-tools. Caller input never establishes identity, authorization, or trust.
5. **Validate at the boundary, trust the core.** At every external edge: `serde` (with closed schemas) and `schemars` frame the bytes, fallible newtype constructors reject structurally invalid values, and explicit core validation enforces semantic and cross-field rules. Deserialization success is never domain validation. Once a value is a validated core type, downstream code trusts it—no re-checking.
6. **MCP surfaces are mandatory and self-describing.** Every capability brick's feature-gated `mcp` handler exposes generated `<namespace>_capabilities` and `<namespace>_schema` tools, even when safe operations are introspection-only. Every project statically combines selected handlers and separately typed `factory_*` project meta-tools into exactly one bounded server binary/process. Generated `schemars` schemas are the description source; do not maintain parallel hand-written schemas. If an agent needs code-reading or multiple project server connections to drive a project, the surface is incomplete.
7. **Single responsibility, small files.** One module owns one concern; a path predicts its contents (`model`, `validation`, `error`, `port`, `service`, adapter modules). Prefer files under ~200 lines. When a file outgrows one clear job, split it along the concern, not arbitrarily.
