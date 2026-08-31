# Rust Factory agent handbook bootstrap

The durable canonical Rust Factory agent handbook is the GitHub Wiki at the exact commit pinned below. This file is a bounded portable bootstrap: it pins and retrieves that handbook, states precedence and failure behavior, and links to canonical pages. It does not duplicate substantive handbook rules.

## Pinned handbook revision

```text
Wiki remote: https://github.com/bannff/Rust-Factory.wiki.git
Wiki SHA: 000f0cf20a0261a497508c5c5af96fbe4a37e352
Tracking issue: https://github.com/bannff/Rust-Factory/issues/42
Kiro compatibility snapshot: main repository commit 0f7db0d770a14d02e878e620293e22eece3e2c1c; all .kiro/steering/**/*.md and .kiro/skills/**/*.md files
```

Retrieve and verify exactly that revision:

```sh
git clone https://github.com/bannff/Rust-Factory.wiki.git rust-factory-wiki
cd rust-factory-wiki
git checkout --detach 000f0cf20a0261a497508c5c5af96fbe4a37e352
test "$(git rev-parse HEAD)" = 000f0cf20a0261a497508c5c5af96fbe4a37e352
test -z "$(git status --porcelain)"
```

Continue only when every command succeeds. The Wiki’s browser pages and branch tip are convenience navigation, never a substitute for this detached revision.

## Canonical pages in the checked-out Wiki

Read `Home.md`, `Handbook-Governance.md`, `Engineering-Principles.md`, `Delivery-Workflow.md`, `Multi-Agent-Coordination.md`, `Architecture.md`, and `Rust-Implementation-Skills.md`. Browser navigation is available at [Home](https://github.com/bannff/Rust-Factory/wiki/Home) and [Handbook Governance](https://github.com/bannff/Rust-Factory/wiki/Handbook-Governance), but is non-authoritative unless it is verified to render the pinned revision.

## Precedence and failure behavior

Apply system, platform, and user instructions first. Then apply executable repository enforcement and repository state: Cargo manifests, `Makefile`, validators, CI, and GitHub protections. Next apply the Wiki handbook at the pinned SHA. Then apply the unchanged Kiro steering and skills compatibility snapshot. Host defaults and floating or unpinned Wiki content are lowest precedence.

At the same level, the more specific applicable instruction governs. Do not silently substitute a Wiki branch tip, another revision, cached content, or host defaults. If the Wiki is unavailable, the SHA cannot be checked out, verification fails, or the Kiro snapshot no longer matches the recorded commit and file set, report the unavailable or mismatched handbook. Continue under system, platform, and user instructions; applicable executable repository enforcement and repository state; then the Kiro compatibility snapshot. Do not claim that the canonical handbook was consulted.
