//! Biscuit cryptographic authorization adapter.
//!
//! This module is the sole home of every `biscuit_auth` type in the crate. It
//! implements [`AuthorizationResolver`](crate::AuthorizationResolver) on top of
//! Biscuit tokens: presented bearer bytes are untrusted caller credentials.
//! Trust is established only after signature verification against a
//! host-injected root public key. The verified authority block's encoded direct
//! facts must contain exactly one string `tenant` and one string `principal`
//! before any Authorizer execution; duplicate facts remain visible there and
//! authority rules cannot substitute for either identity. Datalog authorization
//! then checks the requested [`CapabilityV1`](crate::CapabilityV1) against that
//! same authority. Raw token bytes must never enter logs, errors, or evidence.
//!
//! # Datalog contract (adapter-owned)
//!
//! A minted token's authority block declares, as facts:
//!
//! ```datalog
//! tenant("<tenant-id>");
//! principal("<principal-id>");
//! capability("<capability-wire-name>");   // one per permitted capability
//! allowed_tool("<tool-id>");              // one per granted tool
//! grant("memory");                        // present iff that flag is enabled
//! grant("knowledge");
//! grant("sandbox");
//! grant("communication");
//! ```
//!
//! Authorization runs the policy
//! `allow if capability("<requested-wire>") trusting authority;`. Identity is
//! decoded from the verified authority block through biscuit-auth's generated
//! schema and conversion APIs before the Authorizer is built. Tool and grant
//! extraction queries use `trusting authority` and inherit the same
//! [`AuthorizerLimits`] as authorization. In biscuit-auth 6.0.0 that scope
//! includes the authority block and the authorizer's own facts, but excludes
//! appended attenuation blocks. Appended blocks can therefore restrict
//! authorization through checks, but cannot add or replace tenant, principal,
//! capability, tool, or grant authority facts. The mapping between
//! [`CapabilityV1`](crate::CapabilityV1) and this Datalog is owned entirely
//! here, never leaking into the core.
//!
//! # Fail-closed and adversarial-token defense
//!
//! Every failure — oversized input, a rejected signature, a parse error, an
//! exceeded ceiling, an expired attenuation, or a policy denial — collapses to
//! [`deny_decision`](crate::deny_decision) through [`Self::deny`]. No failure
//! mode is distinguishable from any other in the returned decision, so an
//! adversary learns nothing from a denial.
//!
//! Because the port is synchronous and may run inside an async executor, an
//! adversarial token must not be able to stall a worker. Two layers guard
//! against this:
//!
//! * pre-Datalog structural ceilings ([`MAX_TOKEN_BYTES`], [`MAX_BLOCKS`],
//!   [`MAX_DATALOG_FACTS`], [`MAX_DATALOG_RULES`], [`MAX_DATALOG_CHECKS`]),
//!   checked before the Datalog engine ever runs to a fixpoint; and
//! * the engine's own runtime limits ([`AuthorizerLimits`]) on fact count,
//!   iteration count, and wall-clock time, which bound every `authorize` and
//!   `query` call.
//!
//! # Evidence
//!
//! Verification failures collapse to `Deny` with no distinguishing signal in
//! the decision. This is deliberate. A verification-failure evidence/audit sink
//! is a separately specified follow-up: [`Self::deny`] is the single seam it
//! will attach to, so the deny path is structured to accept such a hook later
//! without changing the public contract. The deny path is not silently
//! swallowed in a way that cannot be wired.

use std::collections::BTreeSet;
use std::time::{Duration, SystemTime};

use biscuit_auth::builder::{Convert, Fact as BuilderFact, Term, fact, string};
use biscuit_auth::builder_ext::BuilderExt;
use biscuit_auth::format::{convert::proto_block_to_token_block, schema};
use biscuit_auth::{
    AuthorizerBuilder, AuthorizerLimits, Biscuit, BlockBuilder, KeyPair, PublicKey, error,
};
use prost::Message;

use crate::{
    AuthContextV1, AuthorizationDecisionV1, AuthorizationRequestV1, AuthorizationResolver,
    CapabilityV1, GrantV1, PrincipalId, TenantId, TokenPresentation, allow_decision, deny_decision,
    verify_decision,
};

/// Maximum accepted length, in bytes, of a presented base64 token.
///
/// Checked before any parsing or cryptography, so an oversized blob is rejected
/// without spending CPU on it.
pub const MAX_TOKEN_BYTES: usize = 8192;
/// Maximum number of blocks (authority plus attenuations) a token may carry.
pub const MAX_BLOCKS: usize = 16;
/// Maximum number of base Datalog facts loaded before authorization runs.
pub const MAX_DATALOG_FACTS: usize = 256;
/// Maximum number of Datalog rules a token may carry.
pub const MAX_DATALOG_RULES: usize = 64;
/// Maximum number of Datalog checks a token may carry.
pub const MAX_DATALOG_CHECKS: usize = 64;

/// Runtime ceiling on fact generation for the Datalog engine.
///
/// This and [`RUNTIME_MAX_ITERATIONS`] are deterministic structural work
/// bounds. The independent wall-clock limit can still deny an otherwise valid
/// evaluation when host scheduling or load consumes its budget.
const RUNTIME_MAX_FACTS: u64 = 512;
/// Runtime ceiling on rule-application iterations for the Datalog engine.
const RUNTIME_MAX_ITERATIONS: u64 = 100;
/// Wall-clock ceiling on any single Datalog execution.
///
/// This is an independent fail-closed backstop, not a service latency SLO or a
/// deterministic work bound. An otherwise valid evaluation can reach it under
/// host load and deny opaquely.
///
/// # Composition-root expectation
///
/// [`BiscuitAuthorizationResolver::authorize`] is synchronous. A host embedding
/// this resolver inside an async executor is responsible for running an
/// `authorize` call via `spawn_blocking` (or the equivalent for its runtime) if
/// it needs to guarantee an executor thread is never held. The resolver does
/// not — and cannot — make that scheduling decision itself; it belongs to the
/// composition root that owns the runtime.
const RUNTIME_MAX_TIME: Duration = Duration::from_secs(1);

const fn adapter_token_size_is_allowed(bytes: usize) -> bool {
    bytes <= MAX_TOKEN_BYTES
}

fn datalog_limits() -> AuthorizerLimits {
    AuthorizerLimits {
        max_facts: RUNTIME_MAX_FACTS,
        max_iterations: RUNTIME_MAX_ITERATIONS,
        max_time: RUNTIME_MAX_TIME,
    }
}

fn direct_authority_identity(biscuit: &Biscuit) -> Option<(TenantId, PrincipalId)> {
    let encoded = biscuit.container().to_proto().authority.block;
    let proto = schema::Block::decode(encoded.as_slice()).ok()?;
    let authority = proto_block_to_token_block(&proto, None).ok()?;

    let facts = authority
        .facts
        .iter()
        .map(|fact| BuilderFact::convert_from(fact, &authority.symbols))
        .collect::<Result<Vec<_>, _>>()
        .ok()?;
    let tenant_id = TenantId::new(exact_direct_string_fact(&facts, "tenant")?).ok()?;
    let principal_id = PrincipalId::new(exact_direct_string_fact(&facts, "principal")?).ok()?;

    Some((tenant_id, principal_id))
}

fn exact_direct_string_fact(facts: &[BuilderFact], name: &str) -> Option<String> {
    let mut values = facts
        .iter()
        .filter(|fact| fact.predicate.name == name)
        .map(|fact| match fact.predicate.terms.as_slice() {
            [Term::Str(value)] => Some(value.clone()),
            _ => None,
        });
    let value = values.next()??;
    values.next().is_none().then_some(value)
}

/// A token-native authorization resolver backed by Biscuit.
///
/// Holds only the host-injected root **public** (verification) key. The key is
/// never read from a request or a token — it is host-owned trust anchored at
/// construction. The struct is immutable and `Send + Sync + 'static`; each call
/// builds its own per-request `Authorizer`, so no verifier state is shared or
/// mutated across calls.
#[derive(Clone, Copy, Debug)]
pub struct BiscuitAuthorizationResolver {
    root_public_key: PublicKey,
}

impl BiscuitAuthorizationResolver {
    /// Constructs a resolver anchored to a host-injected root public key.
    #[must_use]
    pub fn new(root_public_key: PublicKey) -> Self {
        Self { root_public_key }
    }

    /// The single deny seam. Every failure path routes through here so a
    /// verification-failure evidence sink can later attach without altering the
    /// public contract.
    #[allow(clippy::unused_self)]
    fn deny(&self) -> AuthorizationDecisionV1 {
        deny_decision()
    }

    fn authorize_inner(
        &self,
        request: &AuthorizationRequestV1,
        token: &TokenPresentation,
        limits: AuthorizerLimits,
    ) -> Option<AuthorizationDecisionV1> {
        // 1. Pre-verification ceiling: reject an oversized blob before spending
        //    any cryptography or Datalog on it.
        if !adapter_token_size_is_allowed(token.as_str().len()) {
            return None;
        }

        // 2. Cryptographically verify against the injected root public key.
        let biscuit = Biscuit::from_base64(token.as_str(), self.root_public_key).ok()?;

        // 3. Post-parse structural and identity ceilings, still before the
        //    Authorizer is built or executes. Inspect the verified encoded
        //    authority block so duplicate direct facts remain observable and
        //    authority rules cannot synthesize either identity field.
        if biscuit.block_count() > MAX_BLOCKS {
            return None;
        }
        let (tenant_id, principal_id) = direct_authority_identity(&biscuit)?;

        let wire = request.capability.as_str();
        let policy = format!("allow if capability(\"{wire}\") trusting authority;");
        let mut authorizer = AuthorizerBuilder::new()
            .set_limits(limits)
            // Register a `time` fact so expiration checks in attenuation blocks
            // are evaluated rather than silently ignored.
            .time()
            .code(policy)
            .ok()?
            .build(&biscuit)
            .ok()?;

        // 4. Datalog-shape ceilings, measured on the loaded world before the
        //    engine is driven to a fixpoint.
        let (facts, rules, checks, _policies) = authorizer.dump();
        if facts.len() > MAX_DATALOG_FACTS
            || rules.len() > MAX_DATALOG_RULES
            || checks.len() > MAX_DATALOG_CHECKS
        {
            return None;
        }

        // 5. Run the capability gate. A denial or any engine error fails closed.
        authorizer.authorize().ok()?;

        // 6. Derive the capability ceiling from authority facts only. The
        //    Authorizer carries the same limits into each extraction query.
        let tool_ids = string_column(
            &mut authorizer,
            "tool($x) <- allowed_tool($x) trusting authority",
        )?;
        let grants = string_set(&mut authorizer, "g($x) <- grant($x) trusting authority")?;
        let grant = GrantV1::new(
            tool_ids,
            grants.contains("memory"),
            grants.contains("knowledge"),
            grants.contains("sandbox"),
            grants.contains("communication"),
        )
        .ok()?;

        let context = AuthContextV1 {
            tenant_id,
            principal_id,
            request_id: request.request_id.clone(),
            correlation_id: request.correlation_id.clone(),
        };

        let decision = allow_decision(request, &context, &grant).ok()?;
        verify_decision(request, &decision).then_some(decision)
    }

    #[cfg(test)]
    fn authorize_with_limits(
        &self,
        request: &AuthorizationRequestV1,
        token: &TokenPresentation,
        limits: AuthorizerLimits,
    ) -> AuthorizationDecisionV1 {
        self.authorize_inner(request, token, limits)
            .unwrap_or_else(|| self.deny())
    }
}

impl AuthorizationResolver for BiscuitAuthorizationResolver {
    fn authorize(
        &self,
        request: AuthorizationRequestV1,
        token: &TokenPresentation,
    ) -> AuthorizationDecisionV1 {
        self.authorize_inner(&request, token, datalog_limits())
            .unwrap_or_else(|| self.deny())
    }
}

/// Extracts a column of single-term string facts.
fn string_column(authorizer: &mut biscuit_auth::Authorizer, rule: &str) -> Option<Vec<String>> {
    let rows: Vec<(String,)> = authorizer.query(rule).ok()?;
    Some(rows.into_iter().map(|(value,)| value).collect())
}

/// Extracts a set of single-term string facts.
fn string_set(authorizer: &mut biscuit_auth::Authorizer, rule: &str) -> Option<BTreeSet<String>> {
    let rows: Vec<(String,)> = authorizer.query(rule).ok()?;
    Some(rows.into_iter().map(|(value,)| value).collect())
}

/// Privileged host/test-only token minting and attenuation.
///
/// Minting is not part of the [`AuthorizationResolver`](crate::AuthorizationResolver)
/// port and is never exposed through MCP: issuing a token is a trusted host
/// operation, not an agent-drivable one. A private [`KeyPair`] must never be
/// sourced from request data or ordinary configuration, stored in resolver
/// state, placed in logs/errors, or introduced into public core types. These
/// helpers exist only for a privileged host issuer and this crate's tests.
///
/// This adapter provides no revocation mechanism and currently has no
/// verification-failure evidence/audit sink; deployments requiring either must
/// supply separately specified host-owned state and integration.
pub mod mint {
    use super::{
        BlockBuilder, BuilderExt, CapabilityV1, GrantV1, KeyPair, PrincipalId, PublicKey,
        SystemTime, TenantId, error, fact, string,
    };
    use biscuit_auth::Biscuit;

    /// Generates a fresh root keypair for privileged host/test issuance only.
    ///
    /// The private half must remain issuer-owned secret material: it must never
    /// come from request data or ordinary configuration, enter resolver state,
    /// logs, or errors, or cross into public core types. Only the
    /// [`public`](KeyPair::public) half is injected into a resolver.
    #[must_use]
    pub fn generate_root_keypair() -> KeyPair {
        KeyPair::new()
    }

    /// Mints a token carrying a verified identity plus the capability policy and
    /// grant ceiling it permits, returning its URL-safe base64 encoding.
    ///
    /// This is a privileged host/test helper. `root` is private issuer key
    /// material and is subject to the module-level handling restrictions.
    pub fn mint(
        root: &KeyPair,
        tenant_id: &TenantId,
        principal_id: &PrincipalId,
        capabilities: &[CapabilityV1],
        grant: &GrantV1,
    ) -> Result<String, error::Token> {
        let mut builder = Biscuit::builder()
            .fact(fact("tenant", &[string(tenant_id.as_str())]))?
            .fact(fact("principal", &[string(principal_id.as_str())]))?;
        for capability in capabilities {
            builder = builder.fact(fact("capability", &[string(capability.as_str())]))?;
        }
        for tool_id in &grant.allowed_tool_ids {
            builder = builder.fact(fact("allowed_tool", &[string(tool_id)]))?;
        }
        for (enabled, name) in [
            (grant.memory_enabled, "memory"),
            (grant.knowledge_enabled, "knowledge"),
            (grant.sandbox_execution_allowed, "sandbox"),
            (grant.communication_allowed, "communication"),
        ] {
            if enabled {
                builder = builder.fact(fact("grant", &[string(name)]))?;
            }
        }
        builder.build(root)?.to_base64()
    }

    /// Attenuates a token by appending an expiration check.
    ///
    /// This privileged host/test helper treats `token_base64` as secret bearer
    /// credential material and must not log it or include it in errors.
    /// The token is re-verified against `root_public_key` before attenuation,
    /// then a block carrying `check if time($t), $t <= expiry` is appended. The
    /// result still verifies against the same root key.
    pub fn attenuate_expiration(
        token_base64: &str,
        root_public_key: PublicKey,
        expiry: SystemTime,
    ) -> Result<String, error::Token> {
        let token = Biscuit::from_base64(token_base64, root_public_key)?;
        let block = BlockBuilder::new().check_expiration_date(expiry);
        token.append(block)?.to_base64()
    }
}

#[cfg(test)]
mod tests {
    use super::mint::{attenuate_expiration, generate_root_keypair, mint};
    use super::{BiscuitAuthorizationResolver, MAX_BLOCKS, MAX_TOKEN_BYTES};
    use crate::{
        AuthorizationDecisionV1, AuthorizationRequestV1, AuthorizationResolver, CapabilityV1,
        CorrelationId, GrantV1, PrincipalId, RequestId, SafeDenyReasonV1, TenantId,
        TokenPresentation,
    };
    use std::time::{Duration, SystemTime};

    fn tenant() -> TenantId {
        TenantId::new("tenant-a").expect("tenant")
    }
    fn principal() -> PrincipalId {
        PrincipalId::new("principal-a").expect("principal")
    }
    fn grant() -> GrantV1 {
        GrantV1::new(
            ["tool-b".to_owned(), "tool-a".to_owned()],
            true,
            false,
            true,
            false,
        )
        .expect("grant")
    }
    fn request_with(request_id: &str) -> AuthorizationRequestV1 {
        AuthorizationRequestV1 {
            request_id: RequestId::new(request_id).expect("request"),
            correlation_id: CorrelationId::new("correlation").expect("correlation"),
            capability: CapabilityV1::AgentInvoke,
        }
    }

    fn token_str(raw: &str) -> TokenPresentation {
        TokenPresentation::new(raw).expect("token presentation")
    }

    #[test]
    fn resolver_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync + 'static>() {}
        assert_send_sync::<BiscuitAuthorizationResolver>();
    }

    #[test]
    fn valid_token_authorizes_with_identity_grant_and_bound_digest() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let token = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint");

        let decision = resolver.authorize(request_with("request-1"), &token_str(&token));
        let AuthorizationDecisionV1::Allow {
            context,
            effective_grant,
            decision_digest,
        } = decision
        else {
            panic!("expected allow");
        };
        assert_eq!(context.tenant_id.as_str(), "tenant-a");
        assert_eq!(context.principal_id.as_str(), "principal-a");
        assert_eq!(context.request_id.as_str(), "request-1");
        assert_eq!(effective_grant.allowed_tool_ids, ["tool-a", "tool-b"]);
        assert!(effective_grant.memory_enabled);
        assert!(!effective_grant.knowledge_enabled);
        assert!(effective_grant.sandbox_execution_allowed);
        assert!(!effective_grant.communication_allowed);
        assert!(!decision_digest.is_empty());
    }

    #[test]
    fn wrong_root_key_is_denied() {
        let root = generate_root_keypair();
        let other = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(other.public());
        let token = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint");
        assert_eq!(
            resolver.authorize(request_with("request-1"), &token_str(&token)),
            AuthorizationDecisionV1::Deny {
                safe_reason: SafeDenyReasonV1::Denied
            }
        );
    }

    #[test]
    fn tampered_token_is_denied() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let token = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint");
        // Flip the final base64 character to corrupt the signature.
        let mut tampered = token;
        let last = tampered.pop().expect("non-empty token");
        tampered.push(if last == 'A' { 'B' } else { 'A' });
        assert!(matches!(
            resolver.authorize(request_with("request-1"), &token_str(&tampered)),
            AuthorizationDecisionV1::Deny { .. }
        ));
    }

    #[test]
    fn expired_token_is_denied() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let token = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint");
        let expired = attenuate_expiration(
            &token,
            root.public(),
            SystemTime::now() - Duration::from_secs(3600),
        )
        .expect("attenuate");
        assert!(matches!(
            resolver.authorize(request_with("request-1"), &token_str(&expired)),
            AuthorizationDecisionV1::Deny { .. }
        ));
    }

    #[test]
    fn oversized_token_is_denied_by_the_byte_ceiling() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let oversized = "A".repeat(MAX_TOKEN_BYTES + 1);
        assert!(matches!(
            resolver.authorize(request_with("request-1"), &token_str(&oversized)),
            AuthorizationDecisionV1::Deny { .. }
        ));
    }

    #[test]
    fn over_limit_block_count_is_denied() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let mut token = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint");
        // Append far-future (non-expiring) blocks until the block count exceeds
        // the ceiling. The token stays cryptographically valid and unexpired, so
        // the denial is attributable to the block ceiling alone.
        for _ in 0..=MAX_BLOCKS {
            token = attenuate_expiration(
                &token,
                root.public(),
                SystemTime::now() + Duration::from_secs(86_400),
            )
            .expect("attenuate");
        }
        assert!(matches!(
            resolver.authorize(request_with("request-1"), &token_str(&token)),
            AuthorizationDecisionV1::Deny { .. }
        ));
    }

    #[test]
    fn capability_not_permitted_by_token_is_denied() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // Token grants AgentInvoke only; a WorkflowStart request must be denied.
        let token = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint");
        let mut request = request_with("request-1");
        request.capability = CapabilityV1::WorkflowStart;
        assert!(matches!(
            resolver.authorize(request, &token_str(&token)),
            AuthorizationDecisionV1::Deny { .. }
        ));
    }

    #[test]
    fn requests_differing_only_in_request_id_produce_distinct_digests() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let token = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint");

        let first = resolver.authorize(request_with("request-1"), &token_str(&token));
        let second = resolver.authorize(request_with("request-2"), &token_str(&token));
        let (
            AuthorizationDecisionV1::Allow {
                decision_digest: a, ..
            },
            AuthorizationDecisionV1::Allow {
                decision_digest: b, ..
            },
        ) = (first, second)
        else {
            panic!("expected two allows");
        };
        assert_ne!(a, b);
    }

    // --- Adversarial QA additions -------------------------------------------
    //
    // The following tests close fail-closed and identity-integrity gaps in the
    // original suite. They build raw tokens with arbitrary Datalog facts so a
    // failure is attributable to a single cause. `mint` intentionally cannot
    // produce a malformed identity, so these use the raw builder directly.

    /// Builds a raw, correctly-signed token carrying exactly the given
    /// single-term string facts. Used to inject adversarial authority blocks
    /// that `mint` would never produce.
    fn raw_signed_token(root: &super::KeyPair, facts: &[(&str, &str)]) -> String {
        use super::{fact, string};
        let mut builder = super::Biscuit::builder();
        for (name, value) in facts {
            builder = builder
                .fact(fact(name, &[string(value)]))
                .expect("add fact");
        }
        builder
            .build(root)
            .expect("build token")
            .to_base64()
            .expect("encode token")
    }

    fn assert_denied(decision: &AuthorizationDecisionV1) {
        assert_eq!(
            *decision,
            AuthorizationDecisionV1::Deny {
                safe_reason: SafeDenyReasonV1::Denied
            },
            "every failure path must fail closed to Deny"
        );
    }

    #[test]
    fn malformed_base64_is_denied_without_panicking() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // Non-empty, within the byte ceiling, but not a decodable token.
        for garbage in ["!!!not-base64!!!", "not.a.biscuit", "AAAA", "====="] {
            assert_denied(&resolver.authorize(request_with("request-1"), &token_str(garbage)));
        }
    }

    #[test]
    fn token_missing_tenant_fact_is_denied() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // Capability satisfies the gate, principal present, but no tenant fact:
        // identity derivation must find exactly-one and fail closed on zero.
        let token = raw_signed_token(
            &root,
            &[("capability", "agent_invoke"), ("principal", "principal-a")],
        );
        assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&token)));
    }

    #[test]
    fn token_missing_principal_fact_is_denied() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let token = raw_signed_token(
            &root,
            &[("capability", "agent_invoke"), ("tenant", "tenant-a")],
        );
        assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&token)));
    }

    #[test]
    fn token_with_ambiguous_tenant_facts_is_denied() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // Two tenant facts: a resolver that took "the first" would allow tenant
        // confusion. Exactly-one is required, so this must fail closed.
        let token = raw_signed_token(
            &root,
            &[
                ("capability", "agent_invoke"),
                ("principal", "principal-a"),
                ("tenant", "tenant-a"),
                ("tenant", "tenant-b"),
            ],
        );
        assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&token)));
    }

    #[test]
    fn token_with_ambiguous_principal_facts_is_denied() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let token = raw_signed_token(
            &root,
            &[
                ("capability", "agent_invoke"),
                ("tenant", "tenant-a"),
                ("principal", "principal-a"),
                ("principal", "principal-b"),
            ],
        );
        assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&token)));
    }

    #[test]
    fn token_with_identity_violating_id_grammar_is_denied() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // A signed token whose tenant/principal violate the logical-id grammar
        // (uppercase, illegal characters) must not yield an Allow with an
        // unvalidated identity — the newtype constructor rejects it.
        for (tenant_value, principal_value) in [
            ("Tenant-A", "principal-a"),
            ("tenant-a", "Principal-A"),
            ("tenant a", "principal-a"),
            ("tenant-a", "café"),
            ("-leading", "principal-a"),
        ] {
            let token = raw_signed_token(
                &root,
                &[
                    ("capability", "agent_invoke"),
                    ("tenant", tenant_value),
                    ("principal", principal_value),
                ],
            );
            assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&token)));
        }
    }

    #[test]
    fn token_with_tool_grammar_violation_is_denied() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // A verified token can carry a structurally invalid tool id; the grant
        // constructor rejects it and the decision fails closed rather than
        // emitting an Allow with a malformed grant.
        let token = raw_signed_token(
            &root,
            &[
                ("capability", "agent_invoke"),
                ("tenant", "tenant-a"),
                ("principal", "principal-a"),
                ("allowed_tool", "Invalid/Tool"),
            ],
        );
        assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&token)));
    }

    #[test]
    fn over_limit_datalog_facts_is_denied() {
        use super::{MAX_DATALOG_FACTS, fact, string};
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // Build a signed token whose authority block carries more base facts
        // than MAX_DATALOG_FACTS permits. The shape ceiling is measured before
        // the engine runs, so this fails closed deterministically (no reliance
        // on RUNTIME_MAX_TIME).
        let mut builder = super::Biscuit::builder()
            .fact(fact("capability", &[string("agent_invoke")]))
            .expect("capability")
            .fact(fact("tenant", &[string("tenant-a")]))
            .expect("tenant")
            .fact(fact("principal", &[string("principal-a")]))
            .expect("principal");
        for index in 0..=MAX_DATALOG_FACTS {
            let value = format!("tool-{index}");
            builder = builder
                .fact(fact("allowed_tool", &[string(&value)]))
                .expect("tool fact");
        }
        let token = builder
            .build(&root)
            .expect("build")
            .to_base64()
            .expect("encode");
        assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&token)));
    }

    #[test]
    fn over_limit_datalog_checks_is_denied() {
        use super::{MAX_DATALOG_CHECKS, fact, string};
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // Build a signed token whose authority block carries more Datalog
        // `check`s than MAX_DATALOG_CHECKS permits. Identity/capability facts
        // stay valid, no rules are present, and everything lives in the single
        // authority block, so the block-count and facts/rules ceilings are all
        // well under their limits: the denial is attributable to the CHECKS
        // ceiling alone. Checks are the attacker-attenuatable surface (any
        // holder can append a block of them), so this is the security-relevant
        // ceiling. The shape ceiling is measured before the engine runs, so
        // this fails closed deterministically without relying on RUNTIME_MAX_TIME.
        let mut builder = super::Biscuit::builder()
            .fact(fact("capability", &[string("agent_invoke")]))
            .expect("capability")
            .fact(fact("tenant", &[string("tenant-a")]))
            .expect("tenant")
            .fact(fact("principal", &[string("principal-a")]))
            .expect("principal");
        for index in 0..=MAX_DATALOG_CHECKS {
            // Each check is textually distinct so none is folded away; checks
            // are counted per-block without deduplication.
            let check = format!("check if capability(\"cap-{index}\")");
            builder = builder.check(check.as_str()).expect("check");
        }
        let token = builder
            .build(&root)
            .expect("build")
            .to_base64()
            .expect("encode");
        assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&token)));
    }

    #[test]
    fn over_limit_datalog_rules_is_denied() {
        use super::{MAX_DATALOG_RULES, fact, string};
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // Same shape as the checks case, but crossing the RULES ceiling: the
        // authority block carries more Datalog rules than MAX_DATALOG_RULES
        // permits. Identity/capability facts stay valid, no checks are present,
        // and the block count stays at one, so facts/checks/block ceilings are
        // all under their limits and the denial is attributable to the RULES
        // ceiling alone. Distinct rule heads keep each rule individually
        // counted. Measured before the engine runs, so it fails closed
        // deterministically.
        let mut builder = super::Biscuit::builder()
            .fact(fact("capability", &[string("agent_invoke")]))
            .expect("capability")
            .fact(fact("tenant", &[string("tenant-a")]))
            .expect("tenant")
            .fact(fact("principal", &[string("principal-a")]))
            .expect("principal");
        for index in 0..=MAX_DATALOG_RULES {
            // Distinct rule head per iteration so no two rules collapse into
            // one in the loaded world.
            let rule = format!("derived_{index}($x) <- capability($x)");
            builder = builder.rule(rule.as_str()).expect("rule");
        }
        let token = builder
            .build(&root)
            .expect("build")
            .to_base64()
            .expect("encode");
        assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&token)));
    }

    #[test]
    fn identity_is_derived_from_the_token_not_the_request() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // A token minted for tenant-b / principal-b must surface exactly that
        // identity. The request carries no identity, so it cannot influence the
        // outcome: the verified token is the sole identity source.
        let other_tenant = TenantId::new("tenant-b").expect("tenant");
        let other_principal = PrincipalId::new("principal-b").expect("principal");
        let token = mint(
            &root,
            &other_tenant,
            &other_principal,
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint");
        let AuthorizationDecisionV1::Allow { context, .. } =
            resolver.authorize(request_with("request-1"), &token_str(&token))
        else {
            panic!("expected allow");
        };
        assert_eq!(context.tenant_id.as_str(), "tenant-b");
        assert_eq!(context.principal_id.as_str(), "principal-b");
    }

    #[test]
    fn a_token_for_one_tenant_cannot_produce_another_tenants_identity() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // Two distinct tenants, same signing root: each token yields its own
        // tenant and never the other's. Identity cannot be swapped at the
        // request boundary.
        let token_a = mint(
            &root,
            &TenantId::new("tenant-a").expect("tenant"),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint a");
        let token_b = mint(
            &root,
            &TenantId::new("tenant-b").expect("tenant"),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint b");
        for (token, expected) in [(token_a, "tenant-a"), (token_b, "tenant-b")] {
            let AuthorizationDecisionV1::Allow { context, .. } =
                resolver.authorize(request_with("request-1"), &token_str(&token))
            else {
                panic!("expected allow");
            };
            assert_eq!(context.tenant_id.as_str(), expected);
        }
    }

    #[test]
    fn requests_differing_only_in_correlation_id_produce_distinct_digests() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let token = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint");

        let mut first_request = request_with("request-1");
        first_request.correlation_id = CorrelationId::new("correlation-one").expect("correlation");
        let mut second_request = request_with("request-1");
        second_request.correlation_id = CorrelationId::new("correlation-two").expect("correlation");

        let (
            AuthorizationDecisionV1::Allow {
                decision_digest: a, ..
            },
            AuthorizationDecisionV1::Allow {
                decision_digest: b, ..
            },
        ) = (
            resolver.authorize(first_request, &token_str(&token)),
            resolver.authorize(second_request, &token_str(&token)),
        )
        else {
            panic!("expected two allows");
        };
        assert_ne!(a, b);
    }

    #[test]
    fn requests_differing_only_in_capability_produce_distinct_digests() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // A token granting two capabilities lets us compare two allows that
        // differ only in the requested capability: the digests must diverge.
        let token = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke, CapabilityV1::WorkflowStart],
            &grant(),
        )
        .expect("mint");

        let mut invoke = request_with("request-1");
        invoke.capability = CapabilityV1::AgentInvoke;
        let mut start = request_with("request-1");
        start.capability = CapabilityV1::WorkflowStart;

        let (
            AuthorizationDecisionV1::Allow {
                decision_digest: a, ..
            },
            AuthorizationDecisionV1::Allow {
                decision_digest: b, ..
            },
        ) = (
            resolver.authorize(invoke, &token_str(&token)),
            resolver.authorize(start, &token_str(&token)),
        )
        else {
            panic!("expected two allows");
        };
        assert_ne!(a, b);
    }

    #[test]
    fn unexpired_attenuated_token_is_allowed() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // The expiration mechanism must not fail closed on a valid token: an
        // attenuation with a future expiry still authorizes. This is the
        // positive counterpart to `expired_token_is_denied`.
        let token = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint");
        let attenuated = attenuate_expiration(
            &token,
            root.public(),
            SystemTime::now() + Duration::from_secs(3600),
        )
        .expect("attenuate");
        assert!(matches!(
            resolver.authorize(request_with("request-1"), &token_str(&attenuated)),
            AuthorizationDecisionV1::Allow { .. }
        ));
    }

    #[test]
    fn remaining_grant_flags_map_from_token_datalog() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // The valid-token test covers memory+sandbox true; this covers the
        // complementary knowledge+communication true mapping so every grant
        // flag is proven to derive from the token's `grant(...)` facts.
        let complementary =
            GrantV1::new(["tool-a".to_owned()], false, true, false, true).expect("grant");
        let token = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &complementary,
        )
        .expect("mint");
        let AuthorizationDecisionV1::Allow {
            effective_grant, ..
        } = resolver.authorize(request_with("request-1"), &token_str(&token))
        else {
            panic!("expected allow");
        };
        assert!(!effective_grant.memory_enabled);
        assert!(effective_grant.knowledge_enabled);
        assert!(!effective_grant.sandbox_execution_allowed);
        assert!(effective_grant.communication_allowed);
    }

    fn append_block(
        token: &str,
        root_public_key: super::PublicKey,
        block: super::BlockBuilder,
    ) -> String {
        super::Biscuit::from_base64(token, root_public_key)
            .expect("parse token")
            .append(block)
            .expect("append block")
            .to_base64()
            .expect("encode token")
    }

    #[test]
    fn adapter_token_byte_limit_accepts_exact_and_rejects_limit_plus_one() {
        assert!(super::adapter_token_size_is_allowed(MAX_TOKEN_BYTES));
        assert!(!super::adapter_token_size_is_allowed(MAX_TOKEN_BYTES + 1));
    }

    #[test]
    fn valid_token_at_exact_adapter_byte_limit_remains_eligible() {
        use super::{fact, string};

        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let build = |padding_len: usize| {
            super::Biscuit::builder()
                .fact(fact("capability", &[string("agent_invoke")]))
                .expect("capability")
                .fact(fact("tenant", &[string("tenant-a")]))
                .expect("tenant")
                .fact(fact("principal", &[string("principal-a")]))
                .expect("principal")
                .fact(fact("padding", &[string(&"x".repeat(padding_len))]))
                .expect("padding")
                .build(&root)
                .expect("build")
                .to_base64()
                .expect("encode")
        };

        let mut low = 0;
        let mut high = MAX_TOKEN_BYTES;
        while low < high {
            let middle = low + (high - low) / 2;
            if build(middle).len() < MAX_TOKEN_BYTES {
                low = middle + 1;
            } else {
                high = middle;
            }
        }
        let exact = (low.saturating_sub(4)..=low + 4)
            .map(build)
            .find(|token| token.len() == MAX_TOKEN_BYTES)
            .expect("a valid base64 token can land on the exact byte ceiling");

        assert!(matches!(
            resolver.authorize(request_with("request-1"), &token_str(&exact)),
            AuthorizationDecisionV1::Allow { .. }
        ));
    }

    #[test]
    fn block_limit_accepts_exact_and_rejects_limit_plus_one() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let mut token = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint");
        for _ in 1..MAX_BLOCKS {
            token = append_block(
                &token,
                root.public(),
                super::BlockBuilder::new()
                    .check("check if capability(\"agent_invoke\") trusting authority")
                    .expect("check"),
            );
        }
        assert!(matches!(
            resolver.authorize(request_with("request-1"), &token_str(&token)),
            AuthorizationDecisionV1::Allow { .. }
        ));

        token = append_block(
            &token,
            root.public(),
            super::BlockBuilder::new()
                .check("check if capability(\"agent_invoke\") trusting authority")
                .expect("check"),
        );
        assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&token)));
    }

    fn token_with_shape(
        root: &super::KeyPair,
        facts: usize,
        rules: usize,
        checks: usize,
    ) -> String {
        use super::{fact, string};
        let mut builder = super::Biscuit::builder()
            .fact(fact("capability", &[string("agent_invoke")]))
            .expect("capability")
            .fact(fact("tenant", &[string("tenant-a")]))
            .expect("tenant")
            .fact(fact("principal", &[string("principal-a")]))
            .expect("principal");
        for index in 3..facts {
            builder = builder
                .fact(fact("padding", &[string(&format!("p{index}"))]))
                .expect("fact");
        }
        for index in 0..rules {
            builder = builder
                .rule(format!("derived_{index}($x) <- padding($x)").as_str())
                .expect("rule");
        }
        for _ in 0..checks {
            builder = builder
                .check("check if capability(\"agent_invoke\") trusting authority")
                .expect("check");
        }
        builder
            .build(root)
            .expect("build")
            .to_base64()
            .expect("encode")
    }

    #[test]
    fn loaded_fact_limit_accepts_exact_and_rejects_limit_plus_one() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        // The resolver injects one authorizer `time` fact. Therefore 255 token
        // facts load exactly 256, while 256 token facts load 257 and deny.
        for (token_facts, expected_allow) in [
            (super::MAX_DATALOG_FACTS - 1, true),
            (super::MAX_DATALOG_FACTS, false),
        ] {
            let token = token_with_shape(&root, token_facts, 0, 0);
            assert!(token.len() <= MAX_TOKEN_BYTES, "fact token hit byte limit");
            assert_eq!(
                matches!(
                    resolver.authorize(request_with("request-1"), &token_str(&token)),
                    AuthorizationDecisionV1::Allow { .. }
                ),
                expected_allow
            );
        }
    }

    #[test]
    fn loaded_rule_limit_accepts_exact_and_rejects_limit_plus_one() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        for (count, expected_allow) in [
            (super::MAX_DATALOG_RULES, true),
            (super::MAX_DATALOG_RULES + 1, false),
        ] {
            let token = token_with_shape(&root, 3, count, 0);
            assert!(token.len() <= MAX_TOKEN_BYTES, "rule token hit byte limit");
            assert_eq!(
                matches!(
                    resolver.authorize(request_with("request-1"), &token_str(&token)),
                    AuthorizationDecisionV1::Allow { .. }
                ),
                expected_allow
            );
        }
    }

    #[test]
    fn loaded_check_limit_accepts_exact_and_rejects_limit_plus_one() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        for (count, expected_allow) in [
            (super::MAX_DATALOG_CHECKS, true),
            (super::MAX_DATALOG_CHECKS + 1, false),
        ] {
            let token = token_with_shape(&root, 3, 0, count);
            assert!(token.len() <= MAX_TOKEN_BYTES, "check token hit byte limit");
            assert_eq!(
                matches!(
                    resolver.authorize(request_with("request-1"), &token_str(&token)),
                    AuthorizationDecisionV1::Allow { .. }
                ),
                expected_allow
            );
        }
    }

    #[test]
    fn appended_untrusted_facts_cannot_elevate_identity_capability_tools_or_grants() {
        use super::{fact, string};
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let no_capability = raw_signed_token(
            &root,
            &[("tenant", "tenant-a"), ("principal", "principal-a")],
        );
        let elevated_capability = append_block(
            &no_capability,
            root.public(),
            super::BlockBuilder::new()
                .fact(fact("capability", &[string("agent_invoke")]))
                .expect("capability"),
        );
        assert_denied(
            &resolver.authorize(request_with("request-1"), &token_str(&elevated_capability)),
        );

        let base = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &GrantV1::new([], false, false, false, false).expect("grant"),
        )
        .expect("mint");
        let mut block = super::BlockBuilder::new();
        for (name, value) in [
            ("tenant", "tenant-b"),
            ("principal", "principal-b"),
            ("allowed_tool", "elevated-tool"),
            ("grant", "memory"),
            ("grant", "knowledge"),
            ("grant", "sandbox"),
            ("grant", "communication"),
        ] {
            block = block.fact(fact(name, &[string(value)])).expect("fact");
        }
        let elevated = append_block(&base, root.public(), block);
        let AuthorizationDecisionV1::Allow {
            context,
            effective_grant,
            ..
        } = resolver.authorize(request_with("request-1"), &token_str(&elevated))
        else {
            panic!("authority capability should remain allowed");
        };
        assert_eq!(context.tenant_id.as_str(), "tenant-a");
        assert_eq!(context.principal_id.as_str(), "principal-a");
        assert!(effective_grant.allowed_tool_ids.is_empty());
        assert!(!effective_grant.memory_enabled);
        assert!(!effective_grant.knowledge_enabled);
        assert!(!effective_grant.sandbox_execution_allowed);
        assert!(!effective_grant.communication_allowed);
    }

    #[test]
    fn appended_untrusted_rules_cannot_elevate_authority() {
        use super::{fact, string};
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let no_capability = raw_signed_token(
            &root,
            &[("tenant", "tenant-a"), ("principal", "principal-a")],
        );
        let capability_rule = super::BlockBuilder::new()
            .fact(fact("injected_capability", &[string("agent_invoke")]))
            .expect("seed")
            .rule("capability($x) <- injected_capability($x)")
            .expect("rule");
        let elevated = append_block(&no_capability, root.public(), capability_rule);
        assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&elevated)));

        let base = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &GrantV1::new([], false, false, false, false).expect("grant"),
        )
        .expect("mint");
        let mut rights_rule = super::BlockBuilder::new();
        for (predicate, value) in [
            ("injected_tenant", "tenant-b"),
            ("injected_principal", "principal-b"),
            ("injected_tool", "elevated-tool"),
            ("injected_grant", "memory"),
            ("injected_grant", "knowledge"),
            ("injected_grant", "sandbox"),
            ("injected_grant", "communication"),
        ] {
            rights_rule = rights_rule
                .fact(fact(predicate, &[string(value)]))
                .expect("seed");
        }
        for rule in [
            "tenant($x) <- injected_tenant($x)",
            "principal($x) <- injected_principal($x)",
            "allowed_tool($x) <- injected_tool($x)",
            "grant($x) <- injected_grant($x)",
        ] {
            rights_rule = rights_rule.rule(rule).expect("rule");
        }
        let elevated = append_block(&base, root.public(), rights_rule);
        let AuthorizationDecisionV1::Allow {
            context,
            effective_grant,
            ..
        } = resolver.authorize(request_with("request-1"), &token_str(&elevated))
        else {
            panic!("authority capability should remain allowed");
        };
        assert_eq!(context.tenant_id.as_str(), "tenant-a");
        assert_eq!(context.principal_id.as_str(), "principal-a");
        assert!(effective_grant.allowed_tool_ids.is_empty());
        assert!(!effective_grant.memory_enabled);
        assert!(!effective_grant.knowledge_enabled);
        assert!(!effective_grant.sandbox_execution_allowed);
        assert!(!effective_grant.communication_allowed);
    }

    #[test]
    fn appended_checks_restrict_without_elevating() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let base = mint(
            &root,
            &tenant(),
            &principal(),
            &[CapabilityV1::AgentInvoke],
            &grant(),
        )
        .expect("mint");
        let restricted = append_block(
            &base,
            root.public(),
            super::BlockBuilder::new()
                .check("check if capability(\"never-granted\") trusting authority")
                .expect("check"),
        );
        assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&restricted)));
    }

    fn runtime_fact_case(seed_count: usize) -> AuthorizationDecisionV1 {
        use super::{fact, string};
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let mut builder = super::Biscuit::builder()
            .fact(fact("capability", &[string("agent_invoke")]))
            .expect("capability")
            .fact(fact("tenant", &[string("tenant-a")]))
            .expect("tenant")
            .fact(fact("principal", &[string("principal-a")]))
            .expect("principal");
        for index in 0..seed_count {
            builder = builder
                .fact(fact("seed", &[string(&format!("s{index}"))]))
                .expect("seed");
        }
        let token = builder
            .rule("pair($x, $y) <- seed($x), seed($y)")
            .expect("rule")
            .build(&root)
            .expect("build")
            .to_base64()
            .expect("encode");
        resolver.authorize(request_with("request-1"), &token_str(&token))
    }

    #[test]
    fn runtime_max_facts_accepts_bounded_derivation_and_denies_crossing_without_panic() {
        assert!(matches!(
            runtime_fact_case(22),
            AuthorizationDecisionV1::Allow { .. }
        ));
        assert_denied(&runtime_fact_case(23));
    }

    struct RuntimeIterationCase {
        decision: AuthorizationDecisionV1,
        token_bytes: usize,
        blocks: usize,
        facts: usize,
        rules: usize,
        checks: usize,
    }

    fn runtime_iteration_case(edge_count: usize) -> RuntimeIterationCase {
        use super::{fact, string};
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let mut builder = super::Biscuit::builder()
            .fact(fact("capability", &[string("agent_invoke")]))
            .expect("capability")
            .fact(fact("tenant", &[string("tenant-a")]))
            .expect("tenant")
            .fact(fact("principal", &[string("principal-a")]))
            .expect("principal")
            .fact(fact("reached", &[string("step-0")]))
            .expect("seed");
        for index in 0..edge_count {
            builder = builder
                .fact(fact(
                    "edge",
                    &[
                        string(&format!("step-{index}")),
                        string(&format!("step-{}", index + 1)),
                    ],
                ))
                .expect("edge");
        }
        let token = builder
            .rule("reached($next) <- reached($current), edge($current, $next)")
            .expect("recursive rule")
            .build(&root)
            .expect("build");
        let blocks = token.block_count();
        let shape_authorizer = super::AuthorizerBuilder::new()
            .time()
            .build(&token)
            .expect("shape authorizer");
        let (facts, rules, checks, _) = shape_authorizer.dump();
        let encoded = token.to_base64().expect("encode");
        let decision = resolver.authorize_with_limits(
            &request_with("request-1"),
            &token_str(&encoded),
            super::AuthorizerLimits {
                max_facts: super::RUNTIME_MAX_FACTS,
                max_iterations: super::RUNTIME_MAX_ITERATIONS,
                max_time: Duration::from_secs(60),
            },
        );
        RuntimeIterationCase {
            decision,
            token_bytes: encoded.len(),
            blocks,
            facts: facts.len(),
            rules: rules.len(),
            checks: checks.len(),
        }
    }

    fn assert_structurally_bounded(case: &RuntimeIterationCase) {
        assert!(case.token_bytes <= MAX_TOKEN_BYTES);
        assert!(case.blocks <= super::MAX_BLOCKS);
        assert!(case.facts <= super::MAX_DATALOG_FACTS);
        assert!(case.rules <= super::MAX_DATALOG_RULES);
        assert!(case.checks <= super::MAX_DATALOG_CHECKS);
    }

    #[test]
    fn runtime_max_iterations_accepts_bounded_token_and_crossing_denies_opaquely() {
        let accepted = runtime_iteration_case(98);
        assert_structurally_bounded(&accepted);
        assert!(matches!(
            accepted.decision,
            AuthorizationDecisionV1::Allow { .. }
        ));

        let crossing = runtime_iteration_case(100);
        assert_structurally_bounded(&crossing);
        assert_denied(&crossing.decision);
    }

    #[test]
    fn runtime_limits_are_exactly_the_required_values() {
        let limits = super::datalog_limits();
        assert_eq!(limits.max_facts, 512);
        assert_eq!(limits.max_iterations, 100);
        assert_eq!(limits.max_time, Duration::from_secs(1));
    }

    #[test]
    fn parallel_trait_object_resolver_calls_are_isolated() {
        use std::sync::Arc;

        let root = generate_root_keypair();
        let resolver: Arc<dyn AuthorizationResolver> =
            Arc::new(BiscuitAuthorizationResolver::new(root.public()));
        let cases = (0..8)
            .map(|index| {
                let tenant = TenantId::new(format!("tenant-{index}")).expect("tenant");
                let principal = PrincipalId::new(format!("principal-{index}")).expect("principal");
                let grant = GrantV1::new(
                    [format!("tool-{index}")],
                    index % 2 == 0,
                    index % 2 != 0,
                    false,
                    false,
                )
                .expect("grant");
                let token = mint(
                    &root,
                    &tenant,
                    &principal,
                    &[CapabilityV1::AgentInvoke],
                    &grant,
                )
                .expect("mint");
                (index, token)
            })
            .collect::<Vec<_>>();

        let handles = cases
            .into_iter()
            .map(|(index, token)| {
                let resolver = Arc::clone(&resolver);
                std::thread::spawn(move || {
                    resolver.authorize(
                        request_with(&format!("request-{index}")),
                        &token_str(&token),
                    )
                })
            })
            .collect::<Vec<_>>();

        for (index, handle) in handles.into_iter().enumerate() {
            let AuthorizationDecisionV1::Allow {
                context,
                effective_grant,
                ..
            } = handle.join().expect("thread")
            else {
                panic!("expected allow");
            };
            assert_eq!(context.tenant_id.as_str(), format!("tenant-{index}"));
            assert_eq!(context.principal_id.as_str(), format!("principal-{index}"));
            assert_eq!(effective_grant.allowed_tool_ids, [format!("tool-{index}")]);
            assert_eq!(effective_grant.memory_enabled, index % 2 == 0);
            assert_eq!(effective_grant.knowledge_enabled, index % 2 != 0);
        }
    }

    #[test]
    fn unknown_grant_names_add_no_authority() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let token = raw_signed_token(
            &root,
            &[
                ("capability", "agent_invoke"),
                ("tenant", "tenant-a"),
                ("principal", "principal-a"),
                ("grant", "unknown"),
            ],
        );
        let AuthorizationDecisionV1::Allow {
            effective_grant, ..
        } = resolver.authorize(request_with("request-1"), &token_str(&token))
        else {
            panic!("unknown grant must not deny or elevate");
        };
        assert!(!effective_grant.memory_enabled);
        assert!(!effective_grant.knowledge_enabled);
        assert!(!effective_grant.sandbox_execution_allowed);
        assert!(!effective_grant.communication_allowed);
    }

    #[test]
    fn duplicate_identical_authority_identity_facts_are_denied() {
        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());

        for duplicate in ["tenant", "principal"] {
            let mut facts = vec![
                ("capability", "agent_invoke"),
                ("tenant", "tenant-a"),
                ("principal", "principal-a"),
            ];
            facts.push((
                duplicate,
                if duplicate == "tenant" {
                    "tenant-a"
                } else {
                    "principal-a"
                },
            ));
            let token = raw_signed_token(&root, &facts);
            assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&token)));
        }
    }

    #[test]
    fn authority_rules_cannot_substitute_for_required_identity_facts() {
        use super::{fact, string};

        let root = generate_root_keypair();
        let resolver = BiscuitAuthorizationResolver::new(root.public());
        let token = super::Biscuit::builder()
            .fact(fact("capability", &[string("agent_invoke")]))
            .expect("capability")
            .fact(fact(
                "identity_seed",
                &[string("tenant-a"), string("principal-a")],
            ))
            .expect("seed")
            .rule("tenant($tenant) <- identity_seed($tenant, $principal)")
            .expect("tenant rule")
            .rule("principal($principal) <- identity_seed($tenant, $principal)")
            .expect("principal rule")
            .build(&root)
            .expect("build")
            .to_base64()
            .expect("encode");

        assert_denied(&resolver.authorize(request_with("request-1"), &token_str(&token)));
    }
}
