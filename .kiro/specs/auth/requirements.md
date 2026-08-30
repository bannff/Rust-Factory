# Requirements: Auth V1

GitHub [issue #31](https://github.com/bannff/Rust-Factory/issues/31) defines Auth as the token-native authorization capability and `biscuit-auth` as its first cryptographic adapter; its membership in the GitHub project `Software-Factory` was verified at delivery. This specification is **implemented / delivered**. Adversarial QA, security, final Rust SME, and final meta-architecture reviews are **APPROVE** after the direct-authority identity fix, with 41 Biscuit unit tests and 18 public-contract tests passing. Focused formatting, Clippy, and dependency-tree checks pass, and the full combined-tree `make check` passes. `cargo-audit` was unavailable, so no advisory scan is claimed.

The retrospective pre-implementation Rust SME decision remains historical evidence of the corrections it required. Final Rust SME and meta-architecture approval, the full combined-tree quality gate, README synchronization, and package-status promotion to `implemented` establish final delivery.

## 1. Capability boundary

1. Auth SHALL verify an untrusted bearer-token presentation, evaluate the requested closed capability, and return a bounded authorization decision through a synchronous in-process Rust port.
2. The framework-free core SHALL own the public models, validation, canonical decision/grant encoding, digests, decision verification helper, and consumed port. Biscuit types, Datalog, keys, token minting, and attenuation SHALL remain in the feature-gated `biscuit` adapter.
3. `TokenPresentation` is untrusted even when a host extracts and injects it. It carries opaque attacker-controlled bytes and SHALL establish no identity, grant, capability, trust anchor, or decision before cryptographic verification and authorization succeed.
4. `tenant_id` and `principal_id` SHALL be derived only from exactly one validated fact of each kind in the cryptographically verified authority block. Callers supply only `request_id`, `correlation_id`, and the requested `CapabilityV1`.
5. Auth SHALL expose no MCP surface. Authorization and privileged token issuance are not agent-drivable operations.
6. Auth V1 SHALL make no process-boundary, network, transport, persistence, retry, recovery, audit-evidence, or durable authorization claim.

## 2. Exact core port and request scope

`AuthorizationResolver` SHALL remain synchronous, object-safe, and usable as `Arc<dyn AuthorizationResolver>`. It has no associated types, generic methods, async return, or vendor types, and its method is exactly:

```rust
pub trait AuthorizationResolver: Send + Sync {
    fn authorize(
        &self,
        request: AuthorizationRequestV1,
        token: &TokenPresentation,
    ) -> AuthorizationDecisionV1;
}
```

Every parse, signature, trust, expiry, validation, ceiling, runtime-limit, query, or authorization failure SHALL return the same opaque deny decision. The port does not return operational detail.

`AuthorizationRequestV1` SHALL contain exactly a validated `RequestId`, validated `CorrelationId`, and closed `CapabilityV1`. `AuthContextV1` SHALL contain token-derived `TenantId` and `PrincipalId` plus copies of the request and correlation IDs. An allow is invalid unless those copied IDs equal the request IDs used to authorize it.

## 3. Closed V1 models

The closed capability set and stable wire names SHALL be:

- Agent: `agent_definition_validate`, `agent_definition_get`, `agent_definition_list`, `agent_definition_register`, `agent_invoke`.
- Workflow: `workflow_validate`, `workflow_start`, `workflow_get`, `workflow_list`, `workflow_cancel`.
- Evaluation: `evaluation_validate`, `evaluation_evaluate`, `evaluation_get`.
- Observability: `observability_telemetry_query`, `observability_telemetry_status`.
- Memory: `memory_remember`, `memory_recall`, `memory_search`, `memory_forget`, `memory_status`.

These names are permanent V1 digest inputs. Unknown capabilities are unrepresentable through the typed core API.

`GrantV1` SHALL contain exactly an ordered, deduplicated tool allowlist and the booleans `memory_enabled`, `knowledge_enabled`, `sandbox_execution_allowed`, and `communication_allowed`. An empty tool list means no tools. Unknown grant names in a token grant no authority. Tool IDs and grant flags are derived only from the verified authority block.

`AuthorizationDecisionV1` SHALL be closed to:

- `Allow { context, effective_grant, decision_digest }`; and
- `Deny { safe_reason: Denied }`.

`SafeDenyReasonV1` SHALL have exactly one public value, `Denied`. Public construction/validation errors SHALL remain closed to `InvalidId`, `InvalidGrant`, `InvalidToken`, and `LimitExceeded`, with matching `PublicErrorCode` values. Public `Display` and `Debug` SHALL reveal no token, key, signature, Datalog, identity, capability, limit, timing, or backend detail.

## 4. Validation and exact ceilings

| Contract | Exact V1 ceiling |
|---|---:|
| Logical ID bytes | 128 |
| Tool IDs per grant | 64 |
| Tool ID bytes | 128 |
| Core token-presentation bytes | 65,536 |
| Biscuit presented-token bytes before parsing | 8,192 |
| Biscuit blocks, including authority | 16 |
| Loaded Datalog facts before execution | 256 |
| Loaded Datalog rules before execution | 64 |
| Loaded Datalog checks before execution | 64 |
| Authorizer generated facts | 512 |
| Authorizer iterations | 100 |
| Authorizer wall-clock backstop per execution | 1 second |

Logical IDs SHALL contain 1–128 ASCII bytes matching `[a-z0-9][a-z0-9_-]*`. Tool IDs SHALL contain 1–128 ASCII bytes, start and end with an ASCII alphanumeric, and otherwise contain only lowercase ASCII letters, digits, `-`, `_`, or `.`. Grant construction SHALL sort and deduplicate tool IDs before enforcing the 64-distinct-ID ceiling.

The 8,192-byte adapter ceiling is checked before Biscuit parsing or cryptography. Block and loaded-world ceilings are checked after verified parsing but before driving Datalog to a fixpoint. The same runtime limits SHALL cover authorization and every extraction query. Crossing any ceiling fails closed to opaque deny; exact-ceiling inputs remain eligible for normal evaluation.

## 5. Biscuit authority and attenuation contract

The root authority block schema is:

```datalog
tenant("<tenant-id>");
principal("<principal-id>");
capability("<capability-wire-name>");
allowed_tool("<tool-id>");
grant("memory");
grant("knowledge");
grant("sandbox");
grant("communication");
```

There SHALL be exactly one encoded direct authority `tenant(string)` fact and one encoded direct authority `principal(string)` fact. Cardinality SHALL be checked after complete cryptographic verification and before any authorizer execution, using `biscuit-auth`'s authority-block schema/conversion APIs rather than parsing printed Datalog. Duplicate identical direct facts deny, and authority rules deriving either predicate cannot substitute for a missing direct fact. Capabilities, tool IDs, and grant flags may repeat in encoded input but yield canonical sets after validation.

Every authorization policy and every identity/grant extraction query SHALL include explicit `trusting authority` scope. Facts or rules in appended blocks SHALL NOT establish or add tenant, principal, capability, tool, or grant authority. An appended block may attenuate the token through checks, including expiration, but it cannot elevate authority even if it appends facts with the same predicate names. A failed attenuation check denies.

The adapter SHALL verify the complete token chain against a root public key injected by trusted host composition. No request, token fact, environment lookup inside Auth, or MCP input may select or replace that key.

## 6. Minting and key ownership

Minting and attenuation are privileged adapter operations, not methods on `AuthorizationResolver`. The root private key SHALL remain host-owned secret material and SHALL never be stored by the resolver, accepted from an authorization request, serialized into a decision, logged, or exposed through MCP. The resolver retains only the injected root public key.

The mint helper may construct authority facts from already validated core values for trusted host composition and tests. Attenuation uses the token's proof to append restrictive blocks; it does not receive the root private key and cannot rewrite the authority block.

Key generation, secret storage, rotation, compromise recovery, and multi-key selection are composition responsibilities outside V1. A future key-identifier or rotation design requires a separate specification and gate.

## 7. Canonical bytes, digests, and decision verification

Every canonical field SHALL be encoded as ASCII decimal UTF-8 byte length, `:`, the exact UTF-8 bytes, and `\n`. Counts use the same field encoding; booleans are exactly `true` or `false`. SHA-256 is lowercase hexadecimal.

Grant bytes are: `auth-grant-v1`, tool count, ordered tool IDs, memory, knowledge, sandbox, communication.

Allow-decision bytes are: `auth-decision-v1`, tenant, principal, request, correlation, capability wire name, `allow`, `grant-present`, then the canonical grant suffix without a second domain tag. Deny-decision bytes are: `auth-decision-v1`, request, correlation, capability wire name, `deny`, `denied`, `grant-absent`.

`decision_digest(request, decision)` SHALL hash those complete canonical bytes. `allow_decision` SHALL canonicalize the grant, require the context request/correlation IDs to equal the request, and store that digest.

The core SHALL provide this exact verification helper:

```rust
pub fn verify_decision(
    request: &AuthorizationRequestV1,
    decision: &AuthorizationDecisionV1,
) -> bool;
```

For `Allow`, verification returns `true` only when request/correlation scope matches the context, the grant is valid and canonicalizable, the stored digest is exactly 64 lowercase hexadecimal characters, and it equals the recomputed digest. Any changed request, correlation, capability, context identity, grant field, missing/extra digest character, non-lowercase/non-hex digest, or stale digest returns `false`. For the closed opaque `Deny`, verification returns `true`; deny carries no authority-bearing context or grant and authorizes no effect.

The digest is unkeyed defense-in-depth against mismatch or accidental in-process mutation. It is not a signature, MAC, token verification, evidence record, or proof of provenance. A party that constructs a forged decision can recompute a matching digest; therefore consumers SHALL accept authorization only from a trusted injected resolver and SHALL NOT treat `verify_decision` as authenticating caller-supplied or deserialized decisions.

## 8. Runtime and concurrency ownership

The resolver SHALL be immutable, `Send + Sync + 'static`, hold only the root public key, and create fresh per-call authorizer state. It SHALL start no runtime, worker, task, or background thread and SHALL retain no token or decision state between calls.

`authorize` is synchronous and may perform cryptography and bounded Datalog evaluation. An async composition root that must protect executor workers SHALL call it through `spawn_blocking` or the runtime-equivalent blocking boundary. Auth does not choose Tokio, own scheduling, promise cancellation, or claim that the one-second engine backstop is an async deadline. The one-second wall-clock is an independent fail-closed limit: host scheduling or load can consume it and deny an otherwise valid evaluation, so its outcome is not a deterministic work-boundary signal.

## 9. Explicit non-goals

V1 does not migrate any Agent, Workflow, Evaluation, Memory, or Observability consumer from Policy; delete or modify Policy; provide revocation; retain tokens durably; provide a token registry; emit authorization evidence; provide an evidence sink; expose MCP; operate a server or transport; define OAuth/OIDC; rotate keys; distribute trust; or authorize across a process boundary. Each such change requires its own issue, contract, security review, and final gates.

## Sources

- [Issue #31: Auth brick and Biscuit adapter](https://github.com/bannff/Rust-Factory/issues/31)
- [Eclipse Biscuit cryptography and authority-block trust](https://www.biscuitsec.org/docs/reference/cryptography/)
- [Eclipse Biscuit specifications](https://doc.biscuitsec.org/reference/specifications)
- [`biscuit-auth` 6.0.0 API documentation](https://docs.rs/biscuit-auth/6.0.0/biscuit_auth/)
- [`biscuit-auth` source repository](https://github.com/biscuit-auth/biscuit-rust)

External source content was rephrased for compliance with licensing restrictions.
