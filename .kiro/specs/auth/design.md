# Design: Auth V1

This design records the implemented contract for GitHub [issue #31](https://github.com/bannff/Rust-Factory/issues/31), whose membership in the GitHub project `Software-Factory` was verified at delivery. Status is **implemented / delivered**. Adversarial QA, security, final Rust SME, and final meta-architecture reviews are **APPROVE** after the direct-authority identity fix. Evidence is 41 passing Biscuit unit tests plus 18 passing public-contract tests, with focused formatting, Clippy, and dependency-tree checks passing. The full combined-tree `make check` also passes. `cargo-audit` was unavailable, so no advisory scan is claimed.

Final Rust SME and meta-architecture approval, the full combined-tree quality gate, README synchronization, and package-status promotion to `implemented` complete delivery.

## Architecture and core/vendor boundary

```text
trusted host composition
  extracts untrusted bearer bytes
  injects root public key
  owns private keys and blocking scheduling
                 |
                 v
framework-free auth core
  TokenPresentation + request/context + closed capability/grant/decision
  canonical bytes/digests + verify_decision
  AuthorizationResolver
                 ^
                 |
auth::biscuit adapter
  biscuit-auth token verification + bounded Datalog authorization
  privileged mint/attenuation helpers
```

The core depends only on standard-library types and `sha2`. It neither parses Biscuit nor names Datalog, keys, transports, runtimes, MCP, or persistence. `auth::biscuit` is the only module that names `biscuit_auth` types. Vendor key types are permitted only in adapter construction and privileged adapter helpers; they never enter the core port.

`TokenPresentation` is an opaque bounded wrapper, not trusted context. Host extraction prevents an agent from choosing a separate identity field; it does not make bearer bytes trustworthy. The adapter establishes identity only after chain verification, authority-scoped authorization, exact-one identity extraction, core newtype validation, and grant validation all succeed.

## Public core contract

The normative API is the exact synchronous object-safe trait in [requirements.md](requirements.md). The request owns bounded request/correlation scope and one closed capability. The token is borrowed per call, which avoids copying and prevents the resolver from retaining bearer material. The decision returns token-derived identity only on allow.

The port is intentionally infallible. Operational and adversarial failure detail has no safe caller action at this boundary, so every failure becomes the same closed deny. Construction and local model helpers retain the closed `AuthError` taxonomy because they are trusted programming/configuration boundaries, not authorization outcomes.

The closed capability and grant shapes match the current Policy compatibility shape, but that is a migration seam rather than a Policy dependency. Auth neither calls Policy nor authorizes deleting it. Consumer repointing remains separately gated.

## Biscuit authority schema and trust scope

The authority block, signed under the host's root private key, owns these predicates:

```datalog
tenant($tenant)
principal($principal)
capability($capability)
allowed_tool($tool)
grant($grant_name)
```

Authorization selects the requested capability with an authority-scoped rule equivalent to:

```datalog
allow if capability("<closed wire name>") trusting authority;
```

Identity is not selected by a Datalog query. After cryptographic verification and before Authorizer construction/execution, the adapter decodes the verified encoded authority block through `biscuit-auth`'s generated schema and conversion APIs. It requires exactly one direct `tenant(string)` fact and one direct `principal(string)` fact, preserving encoded multiplicity so duplicate identical facts deny and authority rules cannot supply identity. Core newtype constructors validate those direct strings.

Rights extraction remains authority-scoped:

```datalog
tool($x) <- allowed_tool($x) trusting authority
g($x) <- grant($x) trusting authority
```

The explicit trust annotation is mandatory, not documentary. Any holder may append an attenuation block, so an unscoped query could read appended `capability`, `allowed_tool`, or `grant` facts and turn attenuation into elevation. Authority-scoped reads ignore those appended elevation facts. Appended checks remain effective and can only narrow use; expiration is one such check. Appended rules/facts are never a source of base identity or rights.

Unknown grant strings do not become capabilities. Duplicate known facts collapse through canonical set construction. Zero or multiple authority tenant/principal facts deny to prevent ambiguous identity selection.

## Verification sequence

`BiscuitAuthorizationResolver::authorize` performs this order and returns opaque deny on every failed step:

1. Reject a presentation above 8,192 bytes before parsing or cryptography.
2. Parse and verify the complete Biscuit chain against the constructor-injected root public key.
3. Reject more than 16 total blocks.
4. Decode the verified encoded authority block with `biscuit-auth`'s schema/conversion APIs; require exactly one direct string tenant and principal fact, preserving duplicate multiplicity, and validate both core IDs.
5. Build a fresh authorizer with exact fact, iteration, and one-second wall-clock limits; inject verifier time for attenuation checks.
6. Load only the closed requested capability policy, explicitly `trusting authority`.
7. Inspect the loaded world and reject facts above 256, rules above 64, or checks above 64 before fixpoint execution.
8. Authorize; a failed policy or token check denies.
9. Query authority tool/grant facts, canonicalize and validate the grant, and ignore unknown grant names.
10. Copy request/correlation scope into `AuthContextV1`, construct the allow decision, and compute its canonical digest.
11. Verify the constructed decision before returning it; any mismatch denies.

The Authorizer is configured once with `max_facts = 512`, `max_iterations = 100`, and `max_time = 1s`; `biscuit-auth` applies those same stored limits to authorization and the authority-scoped tool/grant extraction queries. Shape ceilings control accepted input; runtime ceilings bound derived work.

## Decision binding and verification

Canonical encoding is length-prefixed so concatenation is unambiguous. Grant tool IDs are sorted and deduplicated before encoding. The domain tags `auth-grant-v1` and `auth-decision-v1` prevent cross-record reuse. The allow digest binds token-derived tenant/principal, request/correlation scope, requested capability, and the complete effective grant. Deny bytes bind request/correlation scope and capability for deterministic diagnostics/tests but carry no stored digest because deny grants no effect.

`verify_decision` is a local consistency check. For allow it first enforces context/request scope equality and canonical grant validity, then validates the digest's exact lowercase-hex shape and compares it with a recomputation. A stale digest, changed field, wrong request, or malformed grant fails. Deny verifies by closed shape and remains non-authoritative.

SHA-256 is deliberately not keyed here. The token signature authenticates the authority block; the digest only catches mismatched use of an already trusted resolver result. A caller able to create an `AuthorizationDecisionV1` can also compute a new digest, so a matching digest is not provenance and cannot rehabilitate an untrusted/deserialized decision. Consumers must invoke an injected resolver and keep authorization checks adjacent to effects.

## Key lifecycle and privileged minting

The host owns root private-key generation, protected storage, access control, rotation, and compromise response. The adapter resolver receives only a root public key at construction and is immutable afterward. Neither token nor request may choose the trust anchor.

Minting is deliberately outside `AuthorizationResolver`. A privileged host-only helper may mint an authority block from validated core values. Its private-key parameter never enters a long-lived resolver. Attenuation re-verifies a token with the public key and uses the token proof to append restrictive checks; it cannot rewrite authority data.

V1 has one injected trust anchor per resolver. Multi-key lookup, key IDs, rotation overlap, hardware keys, remote signing, and key persistence are not implied.

## Resource model

| Layer | Bound | Enforcement point |
|---|---:|---|
| Core presentation | 65,536 bytes | `TokenPresentation::new` |
| Biscuit presentation | 8,192 bytes | before parse/crypto |
| Blocks | 16 | after verified parse |
| Loaded facts | 256 | before Datalog execution |
| Loaded rules | 64 | before Datalog execution |
| Loaded checks | 64 | before Datalog execution |
| Generated facts | 512 | every authorizer execution |
| Iterations | 100 | every authorizer execution |
| Wall-clock | 1 second | every authorizer execution |

Logical IDs, tool IDs, and tool count use the exact core limits in requirements. Exact-limit values proceed; limit plus one denies or returns the applicable construction error. No accepted request path bypasses these ceilings.

The one-second value is an independent fail-closed engine backstop, not a service latency SLO, cancellation contract, deterministic work bound, or deterministic async deadline. It may deny an otherwise valid evaluation when host scheduling or load consumes the wall-clock budget. Deterministic fact and iteration limits provide the primary structural work bounds; tests of those boundaries use a safely nonbinding test-only wall-clock while preserving the production fact and iteration values.

## Dependency decision and vet

The workspace pins `biscuit-auth` to `=6.0.0`; Auth disables default features and explicitly enables only `datalog-macro`:

```toml
biscuit-auth = { workspace = true, optional = true, default-features = false, features = ["datalog-macro"] }
prost = { version = "=0.10.4", optional = true }
```

A focused compile proved that upstream 6.0.0 does not compile its builder modules with every feature disabled: those modules import `ToAnyParam`, while the crate gates that trait behind `datalog-macro`. Enabling that one feature is therefore the minimum correction. The optional exact `prost` pin exposes the `Message` trait required to decode `biscuit-auth`'s public generated authority-block schema after `Biscuit::from_base64` has verified the chain; it resolves to the same 0.10.4 already used by `biscuit-auth` and remains confined to the adapter feature. The `pem` and expanded-regex default features remain disabled.

The inspected normal Auth+Biscuit graph contains exactly one `serde_json`, resolved to workspace-compatible `1.0.145`, and contains no `schemars` or `indexmap`. Therefore it introduces neither a second Schemars major nor `serde_json/preserve_order`. The direct crate declares Apache-2.0. The upstream 6.0.0 manifest declares no `rust-version`; compatibility must not be inferred from metadata. A current workspace-toolchain compile is required in the focused gate and may be recorded only after it passes.

This vet is graph-specific. Any version, feature, or direct-dependency change requires rerunning `cargo tree` checks before approval.

## Errors and denial leakage

Authorization failures intentionally collapse signature failure, malformed encoding, wrong root, expiry, missing/ambiguous identity, policy denial, malformed grant, and every resource-limit failure into `Deny { Denied }`. Returned values and public formatting reveal no distinction. The adapter may retain private implementation detail only if no public `Debug`, `Display`, error source, panic, or decision exposes it.

There is no evidence sink in V1. The single internal deny seam is compatible with a future separately specified sink, but the current design makes no audit, delivery, retention, or durability claim.

## Concurrency and runtime ownership

`BiscuitAuthorizationResolver` stores an immutable copy of the public verification key. Every call allocates independent parser/authorizer state; no mutable state, cache, token, grant, or decision crosses calls. The resolver is `Send + Sync + 'static`, supports `Arc<dyn AuthorizationResolver>`, and starts no threads or tasks.

Because the public port is synchronous, composition decides where it runs. In a Tokio host, authorization belongs behind `spawn_blocking` when blocking an executor worker is unacceptable. Auth does not depend on Tokio, select a runtime, spawn detached work, propagate cancellation, or promise completion before an outer request deadline.

## Deferred boundaries

Policy migration/deletion, consumer adapters, revocation, durable token storage, evidence/audit sinks, MCP, network verification, remote key lookup, multi-key rotation, and process-boundary authorization are outside issue #31. The V1 API is an in-process consumed port only; using it remotely would require a new receiver-derived trust and bounded transport specification.

## Sources

- [Issue #31](https://github.com/bannff/Rust-Factory/issues/31)
- [Eclipse Biscuit cryptography](https://www.biscuitsec.org/docs/reference/cryptography/)
- [Eclipse Biscuit specifications](https://doc.biscuitsec.org/reference/specifications)
- [`biscuit-auth` 6.0.0 documentation](https://docs.rs/biscuit-auth/6.0.0/biscuit_auth/)
- [`biscuit-auth` repository](https://github.com/biscuit-auth/biscuit-rust)

External source content was rephrased for compliance with licensing restrictions.
