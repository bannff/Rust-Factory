#![cfg(feature = "settings")]

use schemars::schema_for;
use serde_json::{Value, json};
use storage::{
    MAX_OBJECTS_GLOBAL, MAX_OBJECTS_PER_TENANT, MAX_VALUE_BYTES_GLOBAL, MAX_VALUE_BYTES_PER_TENANT,
    StorageError,
    settings::{StorageBackend, StorageConfigV1, StorageConfigVersion, StorageSettings},
};

fn valid_config() -> Value {
    json!({
        "version": "v1",
        "backend": "local",
        "max_objects_per_tenant": 1,
        "max_value_bytes_per_tenant": 1,
        "max_objects_global": 1,
        "max_value_bytes_global": 1
    })
}

#[test]
fn wire_names_are_exactly_v1_local_and_redb() {
    assert_eq!(
        serde_json::to_value(StorageConfigVersion::V1).unwrap(),
        json!("v1")
    );
    assert_eq!(
        serde_json::to_value(StorageBackend::Local).unwrap(),
        json!("local")
    );
    assert_eq!(
        serde_json::to_value(StorageBackend::Redb).unwrap(),
        json!("redb")
    );
    assert!(serde_json::from_value::<StorageConfigVersion>(json!("V1")).is_err());
    assert!(serde_json::from_value::<StorageBackend>(json!("memory")).is_err());
}

#[test]
fn config_rejects_unknown_fields_and_missing_or_wrongly_typed_fields() {
    let mut unknown = valid_config();
    unknown["path"] = json!("/secret/store.redb");
    assert!(serde_json::from_value::<StorageConfigV1>(unknown).is_err());

    let mut missing = valid_config();
    missing.as_object_mut().unwrap().remove("backend");
    assert!(serde_json::from_value::<StorageConfigV1>(missing).is_err());

    let mut wrong_type = valid_config();
    wrong_type["max_objects_global"] = json!("1");
    assert!(serde_json::from_value::<StorageConfigV1>(wrong_type).is_err());
}

#[test]
fn generated_schema_is_closed_at_every_object_level_and_enums_are_closed() {
    let schema = serde_json::to_value(schema_for!(StorageConfigV1)).expect("schema serializes");
    assert_eq!(schema["additionalProperties"], json!(false));
    let schema_text = serde_json::to_string(&schema).unwrap();
    assert!(schema_text.contains("\"v1\""));
    assert!(schema_text.contains("\"local\""));
    assert!(schema_text.contains("\"redb\""));
    assert!(!schema_text.contains("path"));
}

#[test]
fn semantic_limits_accept_exact_boundaries() {
    for values in [
        (1, 1, 1, 1),
        (
            MAX_OBJECTS_PER_TENANT,
            MAX_VALUE_BYTES_PER_TENANT,
            MAX_OBJECTS_GLOBAL,
            MAX_VALUE_BYTES_GLOBAL,
        ),
    ] {
        let config = StorageConfigV1 {
            version: StorageConfigVersion::V1,
            backend: StorageBackend::Redb,
            max_objects_per_tenant: values.0,
            max_value_bytes_per_tenant: values.1,
            max_objects_global: values.2,
            max_value_bytes_global: values.3,
        };
        let settings = StorageSettings::try_from(config).expect("boundary must be valid");
        assert_eq!(settings.backend(), StorageBackend::Redb);
        assert_eq!(settings.limits().max_objects_per_tenant(), values.0);
    }
}

#[test]
fn semantic_limits_reject_zero_over_max_and_per_tenant_over_global() {
    let invalid = [
        (0, 1, 1, 1),
        (1, 0, 1, 1),
        (1, 1, 0, 1),
        (1, 1, 1, 0),
        (MAX_OBJECTS_PER_TENANT + 1, 1, MAX_OBJECTS_GLOBAL, 1),
        (1, MAX_VALUE_BYTES_PER_TENANT + 1, 1, MAX_VALUE_BYTES_GLOBAL),
        (1, 1, MAX_OBJECTS_GLOBAL + 1, 1),
        (1, 1, 1, MAX_VALUE_BYTES_GLOBAL + 1),
        (2, 1, 1, 1),
        (1, 2, 1, 1),
    ];
    for values in invalid {
        let config = StorageConfigV1 {
            version: StorageConfigVersion::V1,
            backend: StorageBackend::Local,
            max_objects_per_tenant: values.0,
            max_value_bytes_per_tenant: values.1,
            max_objects_global: values.2,
            max_value_bytes_global: values.3,
        };
        assert_eq!(
            StorageSettings::try_from(config),
            Err(StorageError::InvalidLimits)
        );
    }
}
