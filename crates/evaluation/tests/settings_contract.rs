#![cfg(feature = "settings")]
//! Closed deployment settings and schema contract.

use evaluation::settings::{
    ConfigVersion, EvaluationConfigV1, EvaluationSettings, ExecutorBackendV1, StoreConfigV1,
};
use evaluation::{EvaluationError, MAX_RESULTS_GLOBAL, MAX_RESULTS_PER_TENANT};
use serde_json::json;

#[test]
fn exact_wire_names_decode_and_round_trip() {
    let local: EvaluationConfigV1 = serde_json::from_value(json!({
        "version":"v1", "executor":"local_deterministic",
        "store":{"type":"in_memory","max_results_per_tenant":8,"max_results_global":16}
    }))
    .expect("local wire shape");
    assert_eq!(local.executor, ExecutorBackendV1::LocalDeterministic);
    assert_eq!(
        serde_json::to_value(&local).expect("encode")["executor"],
        "local_deterministic"
    );

    let serdes: EvaluationConfigV1 = serde_json::from_value(json!({
        "version":"v1", "executor":"serdes_ai_evals",
        "store":{"type":"in_memory","max_results_per_tenant":1,"max_results_global":1}
    }))
    .expect("serdes wire shape");
    assert_eq!(serdes.executor, ExecutorBackendV1::SerdesAiEvals);
}

#[test]
fn serde_and_schemars_shapes_are_closed() {
    for value in [
        json!({"version":"v1","executor":"local_deterministic","store":{"type":"in_memory","max_results_per_tenant":1,"max_results_global":1},"typo":true}),
        json!({"version":"v1","executor":"local_deterministic","store":{"type":"in_memory","max_results_per_tenant":1,"max_results_global":1,"typo":true}}),
        json!({"version":"v2","executor":"local_deterministic","store":{"type":"in_memory","max_results_per_tenant":1,"max_results_global":1}}),
        json!({"version":"v1","executor":"unknown","store":{"type":"in_memory","max_results_per_tenant":1,"max_results_global":1}}),
        json!({"version":"v1","executor":"local_deterministic","store":{"type":"unknown"}}),
    ] {
        assert!(serde_json::from_value::<EvaluationConfigV1>(value).is_err());
    }
    let schema = serde_json::to_value(schemars::schema_for!(EvaluationConfigV1)).expect("schema");
    assert_eq!(schema["additionalProperties"], false);
    let encoded = schema.to_string();
    for required in [
        "local_deterministic",
        "serdes_ai_evals",
        "in_memory",
        "max_results_per_tenant",
        "max_results_global",
    ] {
        assert!(encoded.contains(required), "schema omitted {required}");
    }
}

#[test]
fn semantic_capacities_accept_exact_limits_and_reject_invalid_relationships() {
    let config = |per_tenant, global| EvaluationConfigV1 {
        version: ConfigVersion::V1,
        executor: ExecutorBackendV1::LocalDeterministic,
        store: StoreConfigV1::InMemory {
            max_results_per_tenant: per_tenant,
            max_results_global: global,
        },
    };
    let maximum = EvaluationSettings::try_from(config(MAX_RESULTS_PER_TENANT, MAX_RESULTS_GLOBAL))
        .expect("maximum");
    assert_eq!(maximum.max_results_per_tenant(), MAX_RESULTS_PER_TENANT);
    assert_eq!(maximum.max_results_global(), MAX_RESULTS_GLOBAL);
    for pair in [
        (0, 1),
        (1, 0),
        (2, 1),
        (MAX_RESULTS_PER_TENANT + 1, MAX_RESULTS_GLOBAL),
        (1, MAX_RESULTS_GLOBAL + 1),
    ] {
        assert_eq!(
            EvaluationSettings::try_from(config(pair.0, pair.1)),
            Err(EvaluationError::LimitExceeded),
            "{pair:?}"
        );
    }
}
