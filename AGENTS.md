# Rust Factory agent handbook bootstrap

The durable Rust Factory agent handbook is the GitHub Wiki at the exact commit pinned below. This file is a bounded portable bootstrap: it pins and retrieves that handbook, states precedence and failure behavior, and links to canonical pages. It does not duplicate substantive handbook rules.

## Pinned handbook revision

```text
Wiki remote: https://github.com/bannff/Rust-Factory.wiki.git
Wiki SHA: 54bc9397fc9bd2e1a3d781ff579d2c6a239738d0
Tracking issue: https://github.com/bannff/Rust-Factory/issues/42
Kiro compatibility snapshot: updated as part of this working-tree change; no main-repository commit SHA exists yet; scope is all .kiro/steering/**/*.md and .kiro/skills/**/*.md files
```

Retrieve and verify exactly that revision:

```sh
git clone https://github.com/bannff/Rust-Factory.wiki.git rust-factory-wiki
cd rust-factory-wiki
git checkout --detach 54bc9397fc9bd2e1a3d781ff579d2c6a239738d0
test "$(git rev-parse HEAD)" = 54bc9397fc9bd2e1a3d781ff579d2c6a239738d0
test -z "$(git status --porcelain)"
```

Continue only when every command succeeds. The Wiki’s browser pages and branch tip are convenience navigation, never a substitute for this detached revision.

## Canonical pages in the checked-out Wiki

Read `Home.md`, `Handbook-Governance.md`, `Engineering-Principles.md`, `Delivery-Workflow.md`, `Multi-Agent-Coordination.md`, `Architecture.md`, and `Rust-Implementation-Skills.md`. Browser navigation is available at [Home](https://github.com/bannff/Rust-Factory/wiki/Home) and [Handbook Governance](https://github.com/bannff/Rust-Factory/wiki/Handbook-Governance), but is non-authoritative unless it is verified to render the pinned revision.

## Precedence and failure behavior

Apply system, platform, and user instructions first. Then apply executable repository enforcement and repository state: Cargo manifests, `Makefile`, validators, CI, and GitHub protections. Next apply the Wiki handbook at the pinned SHA. Then apply the Kiro steering and skills compatibility snapshot recorded above; during this working-tree change it is identified by scope because no resulting main-repository commit SHA exists yet. Host defaults and floating or unpinned Wiki content are lowest precedence.

At the same level, the more specific applicable instruction governs. Do not silently substitute a Wiki branch tip, another revision, cached content, or host defaults. If the Wiki is unavailable, the SHA cannot be checked out, verification fails, or the Kiro snapshot no longer matches the recorded commit and file set, report the unavailable or mismatched handbook. Continue under system, platform, and user instructions; applicable executable repository enforcement and repository state; then the Kiro compatibility snapshot. Do not claim that the canonical handbook was consulted.
