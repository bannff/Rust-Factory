# Framework and Dependency Policy

No framework belongs in every brick. Cores use typed Rust models, explicit validation, traits, and public errors. Adapters contain transport, provider, persistence, and framework details.

| Boundary | Preferred primitive | Rule |
|---|---|---|
| Semantic canonical input | serde_json exception | permitted in core only for specified canonical semantic input; no serde boundary types in public API |
| Serialization/ingress | serde, serde_json | adapter types and MCP inputs/outputs |
| External schemas/MCP | schemars, rmcp | MCP adapters only |
| Filesystem confinement | cap-std | filesystem adapter only |
| Stable library errors | thiserror candidate | only if repeated error boilerplate justifies it |
| Operational context | anyhow | binaries/adapters only, never public core errors |
| Properties | proptest | state/codec/idempotency invariants when generative coverage adds value |
| Concurrency | loom | synchronization-critical adapters only |
| Fuzzing | cargo-fuzz | parsers/codecs/MCP ingress only |
| Golden vectors | checked-in vectors | canonical codecs/hashes only |
| Async | Tokio | real concurrent-I/O adapters only |

Cargo features default to minimal core behavior. Optional provider/persistence/mesh integrations belong to adapter crates or adapter-owned opt-in features; cores must not expose framework feature choices. Every dependency needs Rust SME approval, exact pin, concrete gap, and contained ownership.

## Portfolio selection and experiments

A project may select a bounded, validated set of named adapter crates for an existing core-owned port. Cargo resolution and explicit constructor injection—not runtime discovery, plugins, or a global registry—compose that selection. Each selected framework stays in its named adapter or experiment crate.

Experiments compare framework adapters behind the same typed port and emit bounded immutable evidence. They do not imply durable evidence, automatic promotion, or a change to core ownership. See `adapter-portfolio.md`.
