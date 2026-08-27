# Design

Each new crate is a deterministic, compile-safe declaration that a capability family exists in the Factory portfolio and has reserved responsibility paths for agent maintenance. It is not a contract implementation.

```text
crates/cache-core/
  Cargo.toml
  src/lib.rs
  src/{model,validation,error,port,service}.rs
  tests/public_contract.rs
```

`lib.rs` includes exactly one private declaration for each reserved module so every responsibility path is compiled. Module and test files contain only blank lines or comments documenting that `status = "scaffolded"` owns no public semantic API. The complete package tree is fixed to the displayed files plus `Cargo.toml`; build scripts, binary/other target trees, and any additional package content are rejected. The package manifest is the authoritative record:

```toml
[package.metadata.rust-factory]
family = "cache"
role = "core"
status = "scaffolded"
```

A repository validator compares root Cargo workspace members, each package metadata record, the exact package tree, status-only source content, absent build/target/features/dependency configuration, and the Vision portfolio registry. Stdlib-only temporary-directory self-tests cover the validator's negative cases. The validator is local/deterministic and fails with actionable diagnostics; it does not derive status from code or emit a runtime API.

This batch creates only family core trees. A later, separately approved semantic specification may replace the corresponding placeholder modules and test with a real core contract, then add only the adapter roles whose prerequisites are met.
