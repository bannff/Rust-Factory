//! The framework-free core's public contract.
//!
//! Nothing here needs an adapter feature, so these tests run in the default build
//! — the build that carries no framework at all. Anything requiring a store lives
//! in `service_contract.rs` or `adapter_contract.rs` behind the `local` feature.
//!
//! Everything is exercised through the public API. A refactor that preserves
//! behaviour leaves this file untouched; a change in behaviour breaks it.

use std::collections::BTreeMap;

use memory::model::Provenance;
use memory::validation::{check_capacity, validate_query, validate_record};
use memory::{
    MAX_CONTENT_BYTES, MAX_QUERY_LIMIT, MemoryError, MemoryKind, MemoryQuery, MemoryRecord,
    Namespace, PublicErrorCode, RecordKey, RunId, TenantId, Timestamp,
};

const NOW: u64 = 1_700_000_000_000_000;

/// A record that satisfies every rule, so a test can perturb one field at a time.
fn valid_record() -> MemoryRecord {
    MemoryRecord {
        tenant_id: TenantId::new("acme").expect("valid tenant"),
        namespace: Namespace::new("notes").expect("valid namespace"),
        key: RecordKey::new("key").expect("valid key"),
        kind: MemoryKind::Factual,
        content: "content".to_owned(),
        tags: Vec::new(),
        metadata: BTreeMap::new(),
        provenance: Some(Provenance {
            run_id: RunId::new("run-1").expect("valid run id"),
            recorded_at: Timestamp::from_micros(NOW),
        }),
    }
}

// ---------------------------------------------------------------- identifiers

#[test]
fn an_identifier_accepts_its_grammar_and_rejects_everything_else() {
    for accepted in ["a", "acme", "acme-1", "acme_1", "acme.one", "9lives"] {
        assert!(
            TenantId::new(accepted).is_ok(),
            "{accepted} is within the grammar"
        );
    }
    for rejected in ["", "Acme", "acme id", "acme/id", "-acme", ".acme", "acme!"] {
        assert_eq!(
            TenantId::new(rejected),
            Err(MemoryError::InvalidId),
            "{rejected:?} must be rejected"
        );
    }
}

#[test]
fn an_oversized_identifier_is_rejected() {
    let too_long = "a".repeat(memory::MAX_ID_BYTES + 1);
    assert_eq!(TenantId::new(too_long), Err(MemoryError::InvalidId));
    let at_limit = "a".repeat(memory::MAX_ID_BYTES);
    assert!(TenantId::new(at_limit).is_ok(), "the limit itself is valid");
}

#[test]
fn every_identifier_type_enforces_the_same_grammar() {
    // The four identifier types are macro-generated, so an instantiation could be
    // wrong while `TenantId`'s tests still pass.
    for rejected in ["", "Bad Name"] {
        assert!(Namespace::new(rejected).is_err(), "namespace {rejected:?}");
        assert!(RecordKey::new(rejected).is_err(), "key {rejected:?}");
        assert!(RunId::new(rejected).is_err(), "run id {rejected:?}");
    }
    let too_long = "a".repeat(memory::MAX_ID_BYTES + 1);
    assert!(Namespace::new(too_long.clone()).is_err());
    assert!(RecordKey::new(too_long.clone()).is_err());
    assert!(RunId::new(too_long).is_err());
    assert_eq!(
        Namespace::new("ok.name-1_2").expect("valid").as_str(),
        "ok.name-1_2"
    );
}

// ------------------------------------------------------------ record validation

#[test]
fn a_valid_record_passes_validation() {
    assert!(validate_record(&valid_record()).is_ok());
    assert!(valid_record().validated().is_ok());
}

#[test]
fn empty_content_is_invalid_and_oversized_content_exceeds_a_limit() {
    let mut record = valid_record();
    record.content = String::new();
    assert_eq!(
        validate_record(&record),
        Err(MemoryError::InvalidRecord),
        "empty content is meaningless"
    );

    record.content = "a".repeat(MAX_CONTENT_BYTES + 1);
    assert_eq!(validate_record(&record), Err(MemoryError::LimitExceeded));

    record.content = "a".repeat(MAX_CONTENT_BYTES);
    assert!(
        validate_record(&record).is_ok(),
        "content exactly at the ceiling is accepted"
    );
}

#[test]
fn content_limits_are_measured_in_bytes_not_characters() {
    // Being explicit about which one governs stops a caller sizing input by
    // `chars().count()` and being refused.
    let mut record = valid_record();
    let multi_byte = "é".repeat(MAX_CONTENT_BYTES / 2);
    assert_eq!(
        multi_byte.len(),
        MAX_CONTENT_BYTES,
        "two bytes per character"
    );
    record.content = multi_byte.clone();
    assert!(
        validate_record(&record).is_ok(),
        "exactly at the byte limit is accepted"
    );

    record.content = format!("{multi_byte}é");
    assert_eq!(
        validate_record(&record),
        Err(MemoryError::LimitExceeded),
        "one character over the byte limit is refused"
    );
}

#[test]
fn a_malformed_or_duplicated_tag_is_rejected() {
    let mut record = valid_record();
    record.tags = vec!["Not A Tag".to_owned()];
    assert_eq!(validate_record(&record), Err(MemoryError::InvalidRecord));

    // A duplicate would make a conjunctive tag filter ambiguous.
    record.tags = vec!["dup".to_owned(), "dup".to_owned()];
    assert_eq!(validate_record(&record), Err(MemoryError::InvalidRecord));

    record.tags = vec!["one".to_owned(), "two".to_owned()];
    assert!(validate_record(&record).is_ok());
}

#[test]
fn record_tag_and_metadata_bounds_are_enforced() {
    let mut record = valid_record();

    record.tags = (0..memory::MAX_TAGS)
        .map(|index| format!("tag-{index}"))
        .collect();
    assert!(
        validate_record(&record).is_ok(),
        "the tag limit itself is allowed"
    );
    record.tags.push("one-more".to_owned());
    assert_eq!(validate_record(&record), Err(MemoryError::LimitExceeded));

    let mut record = valid_record();
    record.metadata = (0..memory::MAX_METADATA_ENTRIES)
        .map(|index| (format!("k{index}"), "v".to_owned()))
        .collect();
    assert!(
        validate_record(&record).is_ok(),
        "the metadata entry limit itself is allowed"
    );
    record
        .metadata
        .insert("one-more".to_owned(), "v".to_owned());
    assert_eq!(validate_record(&record), Err(MemoryError::LimitExceeded));

    let mut record = valid_record();
    record.metadata = BTreeMap::from([(String::new(), "v".to_owned())]);
    assert_eq!(
        validate_record(&record),
        Err(MemoryError::InvalidRecord),
        "an empty metadata key is meaningless"
    );

    record.metadata =
        BTreeMap::from([("k".to_owned(), "v".repeat(memory::MAX_METADATA_ENTRY_BYTES))]);
    assert!(
        validate_record(&record).is_ok(),
        "an entry exactly at the limit is allowed"
    );
    record.metadata = BTreeMap::from([(
        "k".to_owned(),
        "v".repeat(memory::MAX_METADATA_ENTRY_BYTES + 1),
    )]);
    assert_eq!(validate_record(&record), Err(MemoryError::LimitExceeded));
}

// ------------------------------------------------------------- query validation

#[test]
fn a_query_limit_must_be_positive_and_within_the_ceiling() {
    assert_eq!(MemoryQuery::all(0), Err(MemoryError::LimitExceeded));
    assert_eq!(
        MemoryQuery::all(MAX_QUERY_LIMIT + 1),
        Err(MemoryError::LimitExceeded)
    );
    assert!(MemoryQuery::all(MAX_QUERY_LIMIT).is_ok());

    // A caller building the struct directly bypasses `all`, which is why adapters
    // revalidate.
    let unbounded = MemoryQuery {
        limit: u32::MAX,
        ..MemoryQuery::default()
    };
    assert_eq!(
        validate_query(&unbounded),
        Err(MemoryError::LimitExceeded),
        "an unbounded limit is refused wherever it is checked"
    );
}

#[test]
fn an_empty_or_inverted_time_window_is_rejected_rather_than_returning_nothing() {
    let mut query = MemoryQuery::all(8).expect("valid query");
    // `until` is exclusive, so an equal pair can never match. Returning empty
    // would be indistinguishable from missing data.
    query.since = Some(Timestamp::from_micros(100));
    query.until = Some(Timestamp::from_micros(100));
    assert_eq!(validate_query(&query), Err(MemoryError::InvalidQuery));

    query.until = Some(Timestamp::from_micros(99));
    assert_eq!(validate_query(&query), Err(MemoryError::InvalidQuery));

    query.until = Some(Timestamp::from_micros(101));
    assert!(validate_query(&query).is_ok());
}

#[test]
fn a_single_sided_time_window_is_accepted() {
    let mut query = MemoryQuery::all(8).expect("valid");
    query.since = Some(Timestamp::from_micros(1));
    assert!(validate_query(&query).is_ok());

    let mut query = MemoryQuery::all(8).expect("valid");
    query.until = Some(Timestamp::from_micros(1));
    assert!(validate_query(&query).is_ok());
}

#[test]
fn a_duplicated_kind_in_a_query_is_rejected() {
    // Accepted by `contains` but a caller bug, and tolerating it makes the
    // filter's meaning unclear.
    let mut query = MemoryQuery::all(8).expect("valid");
    query.kinds = vec![MemoryKind::Factual, MemoryKind::Factual];
    assert_eq!(validate_query(&query), Err(MemoryError::InvalidQuery));

    query.kinds = vec![MemoryKind::Factual, MemoryKind::Episodic];
    assert!(validate_query(&query).is_ok(), "distinct kinds are fine");
}

#[test]
fn a_kinds_filter_longer_than_the_vocabulary_is_refused() {
    // Bounded before the clone-and-sort, so a caller cannot pay us to sort a huge
    // vector it always intended to be rejected.
    let mut query = MemoryQuery::all(8).expect("valid");
    query.kinds = vec![MemoryKind::Factual; MemoryKind::all().len() + 1];
    assert_eq!(validate_query(&query), Err(MemoryError::InvalidQuery));
}

#[test]
fn query_tag_rules_are_distinguished_from_record_tag_rules() {
    let mut query = MemoryQuery::all(8).expect("valid");
    query.tags = vec!["Not A Tag".to_owned()];
    assert_eq!(
        validate_query(&query),
        Err(MemoryError::InvalidQuery),
        "a malformed tag in a query is an invalid query"
    );

    // The same malformed tag on the write path is an invalid *record*. The codes
    // differ deliberately so a caller can tell which input was at fault.
    let mut record = valid_record();
    record.tags = vec!["Not A Tag".to_owned()];
    assert_eq!(validate_record(&record), Err(MemoryError::InvalidRecord));

    query.tags = (0..=memory::MAX_TAGS)
        .map(|index| format!("tag-{index}"))
        .collect();
    assert_eq!(
        validate_query(&query),
        Err(MemoryError::LimitExceeded),
        "too many tags exceeds a limit rather than being malformed"
    );
}

#[test]
fn query_term_bounds_are_enforced() {
    let mut query = MemoryQuery::all(8).expect("valid");
    query.term = Some(String::new());
    assert_eq!(
        validate_query(&query),
        Err(MemoryError::InvalidQuery),
        "an empty term is meaningless"
    );

    query.term = Some("a".repeat(memory::MAX_TERM_BYTES + 1));
    assert_eq!(validate_query(&query), Err(MemoryError::LimitExceeded));

    query.term = Some("a".repeat(memory::MAX_TERM_BYTES));
    assert!(
        validate_query(&query).is_ok(),
        "a term exactly at the limit is accepted"
    );
}

// -------------------------------------------------------------------- capacity

#[test]
fn the_capacity_rule_bounds_growth_but_never_blocks_a_replace() {
    // A full partition must stay updatable, or it could never be repaired.
    assert!(
        check_capacity(0, true, memory::MAX_PARTITION_RECORDS, false).is_ok(),
        "replacing a key in a full partition is always allowed"
    );
    assert_eq!(
        check_capacity(0, false, memory::MAX_PARTITION_RECORDS, true),
        Err(MemoryError::LimitExceeded),
        "a new key past the record ceiling is refused"
    );
    assert!(
        check_capacity(0, false, memory::MAX_PARTITION_RECORDS - 1, true).is_ok(),
        "one below the ceiling still admits a new key"
    );
    assert_eq!(
        check_capacity(memory::MAX_TENANT_NAMESPACES, true, 0, true),
        Err(MemoryError::LimitExceeded),
        "a new namespace past the namespace ceiling is refused"
    );
    assert!(
        check_capacity(memory::MAX_TENANT_NAMESPACES, false, 0, true).is_ok(),
        "an existing namespace is unaffected by the namespace ceiling"
    );
}

#[test]
fn the_aggregate_record_ceiling_exceeds_the_content_ceiling() {
    // Documented because the per-field limits do not add up to an obvious total,
    // and a caller sizing a transport ceiling needs the real number. A compile-time
    // assertion, because comparing two constants at runtime folds away.
    const _: () = assert!(
        memory::MAX_RECORD_BYTES > MAX_CONTENT_BYTES,
        "metadata and tags mean a record is larger than its content"
    );
    assert_eq!(
        memory::MAX_RECORD_BYTES,
        MAX_CONTENT_BYTES
            + memory::MAX_METADATA_ENTRIES * 2 * memory::MAX_METADATA_ENTRY_BYTES
            + memory::MAX_TAGS * memory::MAX_ID_BYTES
    );
}

// ---------------------------------------------------------------------- errors

#[test]
fn a_tenant_mismatch_is_indistinguishable_from_absence_publicly() {
    // Reporting a distinct code would confirm that a key exists in another
    // tenant, which is itself the leak.
    assert_eq!(
        MemoryError::TenantMismatch.public_code(),
        PublicErrorCode::NotFound
    );
    assert_eq!(
        MemoryError::NotFound.public_code(),
        PublicErrorCode::NotFound
    );
}

#[test]
fn every_error_projects_to_a_stable_public_code() {
    for (error, expected) in [
        (MemoryError::InvalidId, "invalid_id"),
        (MemoryError::InvalidRecord, "invalid_record"),
        (MemoryError::InvalidQuery, "invalid_query"),
        (MemoryError::LimitExceeded, "limit_exceeded"),
        (MemoryError::NotFound, "not_found"),
        (MemoryError::TenantMismatch, "not_found"),
        (MemoryError::AdapterFailure, "adapter_failure"),
    ] {
        assert_eq!(error.public_code().as_str(), expected);
        assert_eq!(
            error.to_string(),
            expected,
            "Display must not reveal more than the public code"
        );
    }
}

#[test]
fn debug_does_not_reintroduce_the_tenant_mismatch_oracle() {
    // `Display` projecting to a public code is not enough on its own: a single
    // `{:?}` in a log line or an error chain would print the variant name and hand
    // back exactly the distinction the projection exists to collapse.
    assert_eq!(
        format!("{:?}", MemoryError::TenantMismatch),
        format!("{:?}", MemoryError::NotFound),
        "Debug must not distinguish a foreign record from a missing one"
    );
    for error in [
        MemoryError::InvalidId,
        MemoryError::InvalidRecord,
        MemoryError::InvalidQuery,
        MemoryError::LimitExceeded,
        MemoryError::NotFound,
        MemoryError::TenantMismatch,
        MemoryError::AdapterFailure,
    ] {
        assert_eq!(
            format!("{error:?}"),
            error.public_code().as_str(),
            "Debug must reveal no more than the public code"
        );
    }
}

// ------------------------------------------------------------------ model rules

#[test]
fn a_memory_kind_has_a_stable_wire_name() {
    // A rename of a variant must not silently change stored or transmitted data.
    assert_eq!(MemoryKind::Factual.as_str(), "factual");
    assert_eq!(MemoryKind::Preference.as_str(), "preference");
    assert_eq!(MemoryKind::Procedural.as_str(), "procedural");
    assert_eq!(MemoryKind::Episodic.as_str(), "episodic");
    assert_eq!(MemoryKind::all().len(), 4, "all() must stay exhaustive");
}

#[test]
fn provenance_can_be_rebuilt_without_any_adapter_feature() {
    // This constructor used to live in the feature-gated agentic module, which
    // made a core capability depend on which adapter happened to be compiled.
    let rebuilt = Provenance::new("run-9", 1_234).expect("valid run id");
    assert_eq!(rebuilt.run_id.as_str(), "run-9");
    assert_eq!(rebuilt.recorded_at.as_micros(), 1_234);
    assert_eq!(
        Provenance::new("Bad Run", 0),
        Err(MemoryError::InvalidId),
        "the run identifier is held to the same grammar as any other"
    );
}

#[test]
fn a_filter_is_conjunctive_across_every_dimension() {
    let mut record = valid_record();
    record.tags = vec!["red".to_owned(), "blue".to_owned()];
    record.content = "alpha beta".to_owned();

    let mut query = MemoryQuery::all(8).expect("valid");
    query.kinds = vec![MemoryKind::Factual];
    query.tags = vec!["red".to_owned(), "blue".to_owned()];
    query.term = Some("beta".to_owned());
    query.since = Some(Timestamp::from_micros(NOW));
    query.until = Some(Timestamp::from_micros(NOW + 1));
    assert!(query.matches(&record), "every filter is satisfied");

    // Each dimension alone is enough to exclude.
    let mut narrowed = query.clone();
    narrowed.kinds = vec![MemoryKind::Episodic];
    assert!(!narrowed.matches(&record), "kind excludes");

    let mut narrowed = query.clone();
    narrowed.tags.push("green".to_owned());
    assert!(!narrowed.matches(&record), "a missing tag excludes");

    let mut narrowed = query.clone();
    narrowed.term = Some("gamma".to_owned());
    assert!(!narrowed.matches(&record), "term excludes");

    let mut narrowed = query.clone();
    narrowed.since = Some(Timestamp::from_micros(NOW + 1));
    assert!(!narrowed.matches(&record), "since is inclusive of NOW only");

    let mut narrowed = query;
    narrowed.until = Some(Timestamp::from_micros(NOW));
    assert!(!narrowed.matches(&record), "until is exclusive");
}

#[test]
fn a_term_filter_is_case_sensitive_and_handles_multi_byte_content() {
    let mut record = valid_record();
    record.content = "Ünïcode Content".to_owned();
    let matching = |term: &str| {
        let mut query = MemoryQuery::all(8).expect("valid");
        query.term = Some(term.to_owned());
        query.matches(&record)
    };
    assert!(matching("Ünïcode"), "a multi-byte term matches");
    assert!(!matching("ünïcode"), "matching is case sensitive");
    assert!(matching("Content"));
}

#[test]
fn a_time_filter_excludes_a_record_that_has_no_provenance() {
    let mut record = valid_record();
    record.provenance = None;

    let mut query = MemoryQuery::all(8).expect("valid");
    assert!(
        query.matches(&record),
        "with no time bound an undated record matches"
    );

    query.since = Some(Timestamp::from_micros(0));
    assert!(
        !query.matches(&record),
        "an undated record cannot satisfy a time bound, so it is excluded"
    );

    let mut query = MemoryQuery::all(8).expect("valid");
    query.until = Some(Timestamp::from_micros(u64::MAX));
    assert!(
        !query.matches(&record),
        "an upper bound alone also excludes an undated record"
    );
}

// -------------------------------------------------------------------- settings

#[cfg(feature = "settings")]
mod settings {
    use memory::settings::{ConfigVersion, MemoryBackend, MemoryConfigV1, MemorySettings};
    use memory::{MAX_QUERY_LIMIT, MemoryError};

    #[test]
    fn a_backend_is_selected_by_a_stable_wire_name() {
        let decoded: MemoryBackend =
            serde_json::from_str("\"agentic_in_process\"").expect("decodes");
        assert_eq!(decoded, MemoryBackend::AgenticInProcess);
        assert_eq!(decoded.as_str(), "agentic_in_process");
        assert_eq!(
            serde_json::to_string(&MemoryBackend::InProcess).expect("encodes"),
            "\"in_process\""
        );
    }

    #[test]
    fn the_backend_vocabulary_does_not_depend_on_enabled_features() {
        // The same project configuration must parse in every binary, so naming a
        // backend this build did not compile is a startup concern, not a parse
        // error.
        for backend in MemoryBackend::all() {
            let encoded = format!("\"{}\"", backend.as_str());
            assert_eq!(
                serde_json::from_str::<MemoryBackend>(&encoded).expect("decodes"),
                backend,
                "{} must always decode",
                backend.as_str()
            );
        }
    }

    #[test]
    fn an_unknown_backend_name_is_refused() {
        assert!(
            serde_json::from_str::<MemoryBackend>(r#""sqlite""#).is_err(),
            "an unrecognised backend must not silently become the default"
        );
    }

    #[test]
    fn a_configuration_decodes_and_validates() {
        let config: MemoryConfigV1 = serde_json::from_str(
            r#"{"version":"v1","backend":"agentic_in_process","default_namespace":"notes","max_query_limit":32}"#,
        )
        .expect("decodes");
        let settings = MemorySettings::from_config(config).expect("validates");
        assert_eq!(settings.backend(), MemoryBackend::AgenticInProcess);
        assert_eq!(settings.default_namespace().as_str(), "notes");
        assert_eq!(settings.max_query_limit(), 32);
    }

    #[test]
    fn an_omitted_backend_defaults_to_the_undemanding_one() {
        let config: MemoryConfigV1 =
            serde_json::from_str(r#"{"version":"v1","default_namespace":"notes"}"#)
                .expect("decodes");
        let settings = MemorySettings::from_config(config).expect("validates");
        assert_eq!(
            settings.backend(),
            MemoryBackend::InProcess,
            "the default must be the backend that claims least"
        );
        assert_eq!(settings.max_query_limit(), 64, "the default limit applies");
    }

    #[test]
    fn an_unknown_field_is_rejected_rather_than_ignored() {
        // A silently ignored typo would select a default the operator did not
        // intend.
        assert!(
            serde_json::from_str::<MemoryConfigV1>(
                r#"{"version":"v1","default_namespace":"notes","backnd":"agentic_in_process"}"#,
            )
            .is_err(),
            "a misspelled field must fail loudly"
        );
    }

    #[test]
    fn decoding_does_not_imply_validity() {
        // Shape and meaning are separate: this decodes cleanly and is still
        // refused, which is the whole reason `MemorySettings` exists.
        let config: MemoryConfigV1 = serde_json::from_str(
            r#"{"version":"v1","default_namespace":"Not A Namespace","max_query_limit":8}"#,
        )
        .expect("decodes");
        assert_eq!(
            MemorySettings::from_config(config),
            Err(MemoryError::InvalidId),
            "a well-formed but meaningless namespace is refused"
        );
    }

    #[test]
    fn a_limit_outside_the_core_ceiling_is_refused() {
        for limit in [0, MAX_QUERY_LIMIT + 1] {
            let config = MemoryConfigV1 {
                version: ConfigVersion::V1,
                backend: MemoryBackend::InProcess,
                default_namespace: "notes".to_owned(),
                max_query_limit: limit,
            };
            assert_eq!(
                MemorySettings::from_config(config),
                Err(MemoryError::LimitExceeded),
                "a deployment cannot set a ceiling of {limit}"
            );
        }
    }

    #[test]
    fn an_absent_or_unknown_version_is_refused() {
        // The discriminator is what stops a future configuration shape being
        // misread as this one, so it must be mandatory and closed.
        assert!(
            serde_json::from_str::<MemoryConfigV1>(r#"{"default_namespace":"notes"}"#).is_err(),
            "version must be mandatory"
        );
        assert!(
            serde_json::from_str::<MemoryConfigV1>(
                r#"{"version":"v2","default_namespace":"notes"}"#
            )
            .is_err(),
            "an unknown version must not fall back to v1"
        );
    }

    #[test]
    fn a_schema_can_be_produced_for_the_configuration() {
        // The schema is what lets a project author validate a file before a binary
        // starts.
        let schema = schemars::schema_for!(MemoryConfigV1);
        let encoded = serde_json::to_string(&schema).expect("encodes");
        assert!(encoded.contains("default_namespace"));
        assert!(encoded.contains("agentic_in_process"));
    }
}
