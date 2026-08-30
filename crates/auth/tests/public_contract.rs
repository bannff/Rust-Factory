//! Public-contract tests for the framework-free auth core.

use auth::{
    AuthContextV1, AuthError, AuthorizationDecisionV1, AuthorizationRequestV1, CapabilityV1,
    CorrelationId, GrantV1, MAX_TOKEN_PRESENTATION_BYTES, MAX_TOOL_ID_BYTES, MAX_TOOL_IDS,
    PrincipalId, PublicErrorCode, RequestId, SafeDenyReasonV1, TenantId, TokenPresentation,
    allow_decision, decision_canonical_bytes, decision_digest, deny_decision,
    grant_canonical_bytes, grant_digest, verify_decision,
};

fn context() -> AuthContextV1 {
    AuthContextV1 {
        tenant_id: TenantId::new("tenant-a").expect("tenant"),
        principal_id: PrincipalId::new("principal-a").expect("principal"),
        request_id: RequestId::new("request-1").expect("request"),
        correlation_id: CorrelationId::new("correlation").expect("correlation"),
    }
}

fn request() -> AuthorizationRequestV1 {
    AuthorizationRequestV1 {
        request_id: RequestId::new("request-1").expect("request"),
        correlation_id: CorrelationId::new("correlation").expect("correlation"),
        capability: CapabilityV1::AgentInvoke,
    }
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

fn allow_bytes(request: &AuthorizationRequestV1, decision: &AuthorizationDecisionV1) -> String {
    String::from_utf8(decision_canonical_bytes(request, decision).expect("bytes")).expect("utf8")
}

#[test]
fn logical_ids_enforce_the_v1_grammar_and_byte_boundary() {
    let valid = "a".repeat(128);
    assert!(TenantId::new(valid.clone()).is_ok());
    assert!(PrincipalId::new(valid.clone()).is_ok());
    assert!(RequestId::new(valid.clone()).is_ok());
    assert!(CorrelationId::new(valid).is_ok());

    for invalid in [
        "",
        "Tenant",
        "-leading",
        "with.dot",
        "with space",
        "café",
        &"a".repeat(129),
    ] {
        assert_eq!(TenantId::new(invalid), Err(AuthError::InvalidId));
        assert_eq!(PrincipalId::new(invalid), Err(AuthError::InvalidId));
        assert_eq!(RequestId::new(invalid), Err(AuthError::InvalidId));
        assert_eq!(CorrelationId::new(invalid), Err(AuthError::InvalidId));
    }
}

#[test]
fn token_presentation_enforces_empty_exact_and_limit_plus_one_bytes() {
    assert_eq!(TokenPresentation::new(""), Err(AuthError::InvalidToken));
    assert_eq!(
        TokenPresentation::new("a".repeat(MAX_TOKEN_PRESENTATION_BYTES))
            .expect("exact ceiling")
            .as_str()
            .len(),
        MAX_TOKEN_PRESENTATION_BYTES
    );
    assert_eq!(
        TokenPresentation::new("a".repeat(MAX_TOKEN_PRESENTATION_BYTES + 1)),
        Err(AuthError::InvalidToken)
    );
    assert_eq!(
        TokenPresentation::new("token").expect("token").as_str(),
        "token"
    );
}

#[test]
fn grants_canonicalize_and_enforce_tool_grammar_and_limits() {
    let canonical = GrantV1::new(
        [
            "tool-b".to_owned(),
            "tool-a".to_owned(),
            "tool-a".to_owned(),
        ],
        false,
        false,
        false,
        false,
    )
    .expect("grant");
    assert_eq!(canonical.allowed_tool_ids, ["tool-a", "tool-b"]);

    let maximum = (0..MAX_TOOL_IDS)
        .map(|index| format!("tool-{index}"))
        .collect::<Vec<_>>();
    assert_eq!(
        GrantV1::new(maximum, false, false, false, false)
            .expect("maximum")
            .allowed_tool_ids
            .len(),
        MAX_TOOL_IDS
    );
    let over_limit = (0..=MAX_TOOL_IDS)
        .map(|index| format!("tool-{index}"))
        .collect::<Vec<_>>();
    assert_eq!(
        GrantV1::new(over_limit, false, false, false, false),
        Err(AuthError::LimitExceeded)
    );
    for invalid in ["", "Tool", "tool/child", &"a".repeat(MAX_TOOL_ID_BYTES + 1)] {
        assert_eq!(
            GrantV1::new([invalid.to_owned()], false, false, false, false),
            Err(AuthError::InvalidGrant)
        );
    }
}

#[test]
fn grant_and_decision_golden_vectors_are_stable() {
    let request = request();
    let allow = allow_decision(&request, &context(), &grant()).expect("allow");

    assert_eq!(
        String::from_utf8(grant_canonical_bytes(&grant()).expect("bytes")).expect("utf8"),
        "13:auth-grant-v1\n1:2\n6:tool-a\n6:tool-b\n4:true\n5:false\n4:true\n5:false\n"
    );
    assert_eq!(
        allow_bytes(&request, &allow),
        "16:auth-decision-v1\n8:tenant-a\n11:principal-a\n9:request-1\n11:correlation\n12:agent_invoke\n5:allow\n13:grant-present\n1:2\n6:tool-a\n6:tool-b\n4:true\n5:false\n4:true\n5:false\n"
    );
    assert_eq!(
        grant_digest(&grant()).expect("digest"),
        "fc6f17fe6a2d68e1af3ee93b140cfa5580d067630393ac0e574053b455d3c204"
    );
    assert_eq!(
        decision_digest(&request, &allow).expect("digest"),
        "8793f07c2507475c01801c51d17954b7e612dbb758291a63d712ba05cdcc5d09"
    );
}

#[test]
fn deny_uses_the_fixed_grant_absent_wire_format() {
    let request = request();
    let deny = deny_decision();
    assert_eq!(
        deny,
        AuthorizationDecisionV1::Deny {
            safe_reason: SafeDenyReasonV1::Denied
        }
    );
    assert_eq!(
        allow_bytes(&request, &deny),
        "16:auth-decision-v1\n9:request-1\n11:correlation\n12:agent_invoke\n4:deny\n6:denied\n12:grant-absent\n"
    );
}

#[test]
fn decision_digest_binds_every_context_field_capability_and_grant() {
    let request = request();
    let decision = allow_decision(&request, &context(), &grant()).expect("allow");
    let original = decision_digest(&request, &decision).expect("digest");

    // Each context field is bound: mutating any one changes the digest.
    let mut altered = decision.clone();
    if let AuthorizationDecisionV1::Allow { context, .. } = &mut altered {
        context.tenant_id = TenantId::new("other-tenant").expect("tenant");
    }
    assert_ne!(
        original,
        decision_digest(&request, &altered).expect("digest")
    );

    altered = decision.clone();
    if let AuthorizationDecisionV1::Allow { context, .. } = &mut altered {
        context.principal_id = PrincipalId::new("other-principal").expect("principal");
    }
    assert_ne!(
        original,
        decision_digest(&request, &altered).expect("digest")
    );

    altered = decision.clone();
    if let AuthorizationDecisionV1::Allow { context, .. } = &mut altered {
        context.request_id = RequestId::new("other-request").expect("request");
    }
    assert_eq!(
        decision_digest(&request, &altered),
        Err(AuthError::InvalidId)
    );

    altered = decision.clone();
    if let AuthorizationDecisionV1::Allow { context, .. } = &mut altered {
        context.correlation_id = CorrelationId::new("other-correlation").expect("correlation");
    }
    assert_eq!(
        decision_digest(&request, &altered),
        Err(AuthError::InvalidId)
    );

    // The capability wire name is bound through the request.
    let mut other_capability = request.clone();
    other_capability.capability = CapabilityV1::WorkflowStart;
    assert_ne!(
        original,
        decision_digest(&other_capability, &decision).expect("digest")
    );

    // The effective grant is bound.
    let narrower = GrantV1::new(["tool-a".to_owned()], true, false, true, false).expect("grant");
    let changed = allow_decision(&request, &context(), &narrower).expect("allow");
    assert_ne!(
        original,
        decision_digest(&request, &changed).expect("digest")
    );
}

#[test]
fn every_capability_wire_name_is_length_prefixed_into_the_digest() {
    for (capability, name) in [
        (
            CapabilityV1::AgentDefinitionValidate,
            "agent_definition_validate",
        ),
        (CapabilityV1::AgentDefinitionGet, "agent_definition_get"),
        (CapabilityV1::AgentDefinitionList, "agent_definition_list"),
        (
            CapabilityV1::AgentDefinitionRegister,
            "agent_definition_register",
        ),
        (CapabilityV1::AgentInvoke, "agent_invoke"),
        (CapabilityV1::WorkflowValidate, "workflow_validate"),
        (CapabilityV1::WorkflowStart, "workflow_start"),
        (CapabilityV1::WorkflowGet, "workflow_get"),
        (CapabilityV1::WorkflowList, "workflow_list"),
        (CapabilityV1::WorkflowCancel, "workflow_cancel"),
        (CapabilityV1::EvaluationValidate, "evaluation_validate"),
        (CapabilityV1::EvaluationEvaluate, "evaluation_evaluate"),
        (CapabilityV1::EvaluationGet, "evaluation_get"),
        (
            CapabilityV1::ObservabilityTelemetryQuery,
            "observability_telemetry_query",
        ),
        (
            CapabilityV1::ObservabilityTelemetryStatus,
            "observability_telemetry_status",
        ),
        (CapabilityV1::MemoryRemember, "memory_remember"),
        (CapabilityV1::MemoryRecall, "memory_recall"),
        (CapabilityV1::MemorySearch, "memory_search"),
        (CapabilityV1::MemoryForget, "memory_forget"),
        (CapabilityV1::MemoryStatus, "memory_status"),
    ] {
        assert_eq!(capability.as_str(), name);
        let mut request = request();
        request.capability = capability;
        let decision = allow_decision(&request, &context(), &grant()).expect("allow");
        assert!(
            allow_bytes(&request, &decision).contains(&format!("{}:{name}\n", name.len())),
            "wire name must be length-prefixed into evidence"
        );
    }
}

fn replace_digest(request: &AuthorizationRequestV1, decision: &mut AuthorizationDecisionV1) {
    let digest = decision_digest(request, decision).expect("digest");
    let AuthorizationDecisionV1::Allow {
        decision_digest, ..
    } = decision
    else {
        panic!("expected allow");
    };
    *decision_digest = digest;
}

#[test]
fn public_error_taxonomy_has_exactly_the_four_v1_codes() {
    assert_eq!(
        [
            AuthError::InvalidId.public_code(),
            AuthError::InvalidGrant.public_code(),
            AuthError::InvalidToken.public_code(),
            AuthError::LimitExceeded.public_code(),
        ],
        [
            PublicErrorCode::InvalidId,
            PublicErrorCode::InvalidGrant,
            PublicErrorCode::InvalidToken,
            PublicErrorCode::LimitExceeded,
        ]
    );
}

#[test]
fn allow_decision_rejects_request_and_correlation_scope_mismatch_as_invalid_id() {
    let request = request();
    let mut mismatched = context();
    mismatched.request_id = RequestId::new("request-2").expect("request");
    assert_eq!(
        allow_decision(&request, &mismatched, &grant()),
        Err(AuthError::InvalidId)
    );

    mismatched = context();
    mismatched.correlation_id = CorrelationId::new("other-correlation").expect("correlation");
    assert_eq!(
        allow_decision(&request, &mismatched, &grant()),
        Err(AuthError::InvalidId)
    );
}

#[test]
fn verify_decision_accepts_valid_allow_and_closed_deny() {
    let request = request();
    assert!(verify_decision(
        &request,
        &allow_decision(&request, &context(), &grant()).expect("allow")
    ));
    assert!(verify_decision(&request, &deny_decision()));
}

#[test]
fn verify_decision_rejects_request_correlation_capability_and_context_identity_mutation() {
    let request = request();
    let decision = allow_decision(&request, &context(), &grant()).expect("allow");

    let mut changed_request = request.clone();
    changed_request.request_id = RequestId::new("request-2").expect("request");
    assert!(!verify_decision(&changed_request, &decision));

    changed_request = request.clone();
    changed_request.correlation_id = CorrelationId::new("other-correlation").expect("correlation");
    assert!(!verify_decision(&changed_request, &decision));

    changed_request = request.clone();
    changed_request.capability = CapabilityV1::WorkflowStart;
    assert!(!verify_decision(&changed_request, &decision));

    let mut changed_decision = decision.clone();
    let AuthorizationDecisionV1::Allow { context, .. } = &mut changed_decision else {
        panic!("expected allow");
    };
    context.tenant_id = TenantId::new("tenant-b").expect("tenant");
    assert!(!verify_decision(&request, &changed_decision));

    changed_decision = decision;
    let AuthorizationDecisionV1::Allow { context, .. } = &mut changed_decision else {
        panic!("expected allow");
    };
    context.principal_id = PrincipalId::new("principal-b").expect("principal");
    assert!(!verify_decision(&request, &changed_decision));
}

#[test]
fn verify_decision_rejects_every_grant_boolean_and_tool_mutation() {
    let request = request();
    let decision = allow_decision(&request, &context(), &grant()).expect("allow");

    for mutate in [
        |grant: &mut GrantV1| grant.memory_enabled = !grant.memory_enabled,
        |grant: &mut GrantV1| grant.knowledge_enabled = !grant.knowledge_enabled,
        |grant: &mut GrantV1| {
            grant.sandbox_execution_allowed = !grant.sandbox_execution_allowed;
        },
        |grant: &mut GrantV1| grant.communication_allowed = !grant.communication_allowed,
    ] {
        let mut changed = decision.clone();
        let AuthorizationDecisionV1::Allow {
            effective_grant, ..
        } = &mut changed
        else {
            panic!("expected allow");
        };
        mutate(effective_grant);
        assert!(!verify_decision(&request, &changed));
    }

    let mut changed = decision;
    let AuthorizationDecisionV1::Allow {
        effective_grant, ..
    } = &mut changed
    else {
        panic!("expected allow");
    };
    effective_grant.allowed_tool_ids.push("tool-c".to_owned());
    assert!(!verify_decision(&request, &changed));
}

#[test]
fn verify_decision_rejects_directly_constructed_noncanonical_invalid_and_over_limit_grants() {
    let request = request();
    let valid = allow_decision(&request, &context(), &grant()).expect("allow");
    let malformed_tools = [
        vec!["tool-b".to_owned(), "tool-a".to_owned()],
        vec!["tool-a".to_owned(), "tool-a".to_owned()],
        vec!["Invalid/Tool".to_owned()],
        (0..=MAX_TOOL_IDS)
            .map(|index| format!("tool-{index}"))
            .collect(),
    ];

    for allowed_tool_ids in malformed_tools {
        let mut malformed = valid.clone();
        let AuthorizationDecisionV1::Allow {
            effective_grant, ..
        } = &mut malformed
        else {
            panic!("expected allow");
        };
        effective_grant.allowed_tool_ids = allowed_tool_ids;
        assert!(!verify_decision(&request, &malformed));
    }
}

#[test]
fn verify_decision_rejects_empty_short_long_uppercase_nonhex_and_stale_digests() {
    let request = request();
    let valid = allow_decision(&request, &context(), &grant()).expect("allow");
    let AuthorizationDecisionV1::Allow {
        decision_digest: valid_digest,
        ..
    } = &valid
    else {
        panic!("expected allow");
    };
    for malformed in [
        String::new(),
        "a".repeat(63),
        "a".repeat(65),
        valid_digest.to_ascii_uppercase(),
        "g".repeat(64),
        "0".repeat(64),
    ] {
        let mut changed = valid.clone();
        let AuthorizationDecisionV1::Allow {
            decision_digest, ..
        } = &mut changed
        else {
            panic!("expected allow");
        };
        *decision_digest = malformed;
        assert!(!verify_decision(&request, &changed));
    }
}

#[test]
fn recomputed_digest_proves_consistency_not_provenance() {
    let request = request();
    let mut caller_constructed = AuthorizationDecisionV1::Allow {
        context: AuthContextV1 {
            tenant_id: TenantId::new("forged-tenant").expect("tenant"),
            principal_id: PrincipalId::new("forged-principal").expect("principal"),
            request_id: request.request_id.clone(),
            correlation_id: request.correlation_id.clone(),
        },
        effective_grant: GrantV1::new(["forged-tool".to_owned()], true, true, true, true)
            .expect("grant"),
        decision_digest: String::new(),
    };
    replace_digest(&request, &mut caller_constructed);

    assert!(verify_decision(&request, &caller_constructed));
    // Passing verifies internal consistency only. A consumer must still reject
    // this caller-constructed value because it did not come from its trusted
    // injected AuthorizationResolver.
}

#[test]
fn tool_ids_cover_empty_exact_and_limit_plus_one_boundaries() {
    assert!(
        GrantV1::new([], false, false, false, false)
            .expect("empty grant")
            .allowed_tool_ids
            .is_empty()
    );
    let exact = format!("a{}z", "x".repeat(MAX_TOOL_ID_BYTES - 2));
    assert_eq!(
        GrantV1::new([exact.clone()], false, false, false, false)
            .expect("exact tool id")
            .allowed_tool_ids,
        [exact]
    );
    let over = format!("a{}z", "x".repeat(MAX_TOOL_ID_BYTES - 1));
    assert_eq!(
        GrantV1::new([over], false, false, false, false),
        Err(AuthError::InvalidGrant)
    );
}

#[test]
fn deny_digest_golden_vector_is_stable() {
    assert_eq!(
        decision_digest(&request(), &deny_decision()).expect("digest"),
        "27c9f70620b94844c3df4ababd59f53861333f7c507e46edc00ca2c298ac30a4"
    );
}

#[test]
fn public_error_and_token_formatting_do_not_leak_input() {
    let secret = "secret-token-material";
    let token = TokenPresentation::new(secret).expect("token");
    assert!(!format!("{token:?}").contains(secret));
    for error in [
        AuthError::InvalidId,
        AuthError::InvalidGrant,
        AuthError::InvalidToken,
        AuthError::LimitExceeded,
    ] {
        assert!(!format!("{error}").contains(secret));
        assert!(!format!("{error:?}").contains(secret));
    }
}
