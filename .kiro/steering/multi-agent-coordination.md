---
inclusion: always
---
# Multi-Agent Coordination

Multiple agents scaffold different bricks at the same time in one shared working directory. These rules keep them from colliding without git worktrees.

1. **One brick per agent.** Claim exactly one `crates/<brick>/` folder and do all your work there. Do not edit another brick's folder. If your task needs a change in someone else's brick, stop and flag it rather than reaching in.
2. **Tracking first.** Before any package is created, the brick SHALL have its own GitHub issue and GitHub Projects roadmap entry identifying the family and intended owning crate.
3. **Shared files are setup-only.** A dedicated setup agent, never a brick-scaffolding agent mid-flight, registers the package in the root `Cargo.toml` workspace members, the `Makefile` quality matrix, the validator's adapter mapping and status-only allowlist as applicable, and matching validator tests. After setup, brick folders are disjoint and agents can run in parallel without merge conflicts.
4. **Shared ownership is strict.** Brick agents do not edit root `Cargo.toml`, `Makefile`, `scripts/validate_brick_registry.py`, or `scripts/test_validate_brick_registry.py`. If setup needs to change one of those files, the setup agent preserves existing members, matrix coverage, mappings, allowlists, and tests unrelated to the new brick.
5. **Builds share one `target/`.** Cargo locks it, so a simultaneous `make check` waits its turn — expected, not an error. Do not disable or work around the lock.
