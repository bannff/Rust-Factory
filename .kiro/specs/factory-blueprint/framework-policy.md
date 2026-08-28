# Framework and Dependency Policy

No framework belongs in every brick. Cores use typed Rust models, explicit validation, traits, and public errors. Adapters contain transport, provider, persistence, and framework details.

| Boundary | Preferred primitive | Rule |
|---|---|---|
| Semantic canonical input | serde_json exception | permitted in core only for specified canonical semantic input; no serde boundary types in public API |
| Serialization/ingress | serde, serde_json | closed adapter DTOs (`deny_unknown_fields` unless an extension map is specified), private conversion, and explicit raw/semantic bounds |
| External schemas/MCP | schemars, rmcp | discovery/shape documentation at MCP adapters only; schema success is not domain validation or authorization |
| Declarative selection | serde, schemars | `settings` module only; owns configuration shape, never the configuration source or the selection-to-constructor `match` |
| Agent memory backend | agentic-memory `=0.4.2` | `memory`'s `agentic` module only, `default-features = false`; no vendor type in a public signature. See [memory](../memory/requirements.md) section 7 |
| Filesystem confinement | cap-std | filesystem adapter only |
| Stable library errors | thiserror candidate | only if repeated error boilerplate justifies it |
| Operational context | anyhow | binaries/adapters only, never public core errors |
| Properties | proptest | state/codec/idempotency invariants when generative coverage adds value |
| Concurrency | loom | synchronization-critical adapters only |
| Fuzzing | cargo-fuzz | parsers/codecs/MCP ingress only |
| Golden vectors | checked-in vectors | canonical codecs/hashes only |
| Async | Tokio | real concurrent-I/O adapters only |

Cargo features default to minimal core behavior. Optional provider/persistence/mesh integrations belong to adapter crates or adapter-owned opt-in features; cores must not expose framework feature choices. Every dependency needs Rust SME approval, exact pin, concrete gap, and contained ownership.

## Transport and composition boundaries

`mcp-transport` is an adapter-only crate when repeated bounded framing behavior has at least two consumers. It may contain rmcp, Tokio, futures, and codec dependencies; cores may not. Binaries under `projects/` are composition roots and own runtime lifecycle, transport binding, configuration, host-derived trusted context, Policy construction, concrete adapter selection, logging initialization, and shutdown. A brick's `mcp` module is a reusable bounded adapter: it accepts an injected transport/service lifecycle and must not read stdio, construct `BoundedStdioTransport`, or choose Tokio process lifecycle. That migration is complete — `serve_stdio` has been deleted from every brick, so no library owns process lifecycle and no brick declares a direct `tokio` dependency. Internal execution remains typed Rust port calls; remote client adapters and async-port migrations require separate demonstrated-need specifications.

## Portfolio selection and experiments

A project may select a bounded, validated set of named adapter crates for an existing core-owned port. Cargo resolution and explicit constructor injection—not runtime discovery, plugins, or a global registry—compose that selection. Each selected framework stays in its named adapter or experiment crate.

Experiments compare framework adapters behind the same typed port and emit bounded immutable evidence. They do not imply durable evidence, automatic promotion, or a change to core ownership. See `adapter-portfolio.md`.
