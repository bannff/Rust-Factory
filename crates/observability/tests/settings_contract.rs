#![cfg(feature = "settings")]

use observability::settings::{
    ObservabilityConfigV1, ObservabilitySettings, SinkConfigV1, SinkSettings,
};
use observability::{MAX_LOCAL_EVENTS_PER_TENANT, MAX_QUERY_LIMIT, ObservabilityError};
use serde_json::json;

#[test]
fn serde_and_schema_shapes_are_closed_and_backend_names_are_stable() {
    for value in [
        json!({"version":1,"sink":{"type":"local","max_events_per_tenant":8},"max_query_limit":4,"typo":true}),
        json!({"version":1,"sink":{"type":"local","max_events_per_tenant":8,"typo":true},"max_query_limit":4}),
        json!({"version":1,"sink":{"type":"unknown"},"max_query_limit":4}),
    ] {
        assert!(serde_json::from_value::<ObservabilityConfigV1>(value).is_err());
    }
    let schema =
        serde_json::to_value(schemars::schema_for!(ObservabilityConfigV1)).expect("schema");
    assert_eq!(schema["additionalProperties"], false);
    let encoded = schema.to_string();
    assert!(encoded.contains("open_telemetry_logs"));
    assert!(encoded.contains("max_events_per_tenant"));
}

#[test]
fn version_semantic_limits_and_backend_selection_are_validated() {
    let local = ObservabilitySettings::try_from(ObservabilityConfigV1 {
        version: 1,
        sink: SinkConfigV1::Local {
            max_events_per_tenant: MAX_LOCAL_EVENTS_PER_TENANT,
        },
        max_query_limit: MAX_QUERY_LIMIT,
    })
    .expect("maximum settings");
    assert_eq!(
        local,
        ObservabilitySettings {
            sink: SinkSettings::Local {
                max_events_per_tenant: MAX_LOCAL_EVENTS_PER_TENANT
            },
            max_query_limit: MAX_QUERY_LIMIT
        }
    );
    let otel = ObservabilitySettings::try_from(ObservabilityConfigV1 {
        version: 1,
        sink: SinkConfigV1::OpenTelemetryLogs,
        max_query_limit: 1,
    })
    .expect("otel");
    assert_eq!(otel.sink, SinkSettings::OpenTelemetryLogs);

    assert_eq!(
        ObservabilitySettings::try_from(ObservabilityConfigV1 {
            version: 2,
            sink: SinkConfigV1::OpenTelemetryLogs,
            max_query_limit: 1
        }),
        Err(ObservabilityError::InvalidConfiguration)
    );
    for limit in [0, MAX_QUERY_LIMIT + 1] {
        assert_eq!(
            ObservabilitySettings::try_from(ObservabilityConfigV1 {
                version: 1,
                sink: SinkConfigV1::OpenTelemetryLogs,
                max_query_limit: limit
            }),
            Err(ObservabilityError::LimitExceeded)
        );
    }
    for capacity in [0, MAX_LOCAL_EVENTS_PER_TENANT + 1] {
        assert_eq!(
            ObservabilitySettings::try_from(ObservabilityConfigV1 {
                version: 1,
                sink: SinkConfigV1::Local {
                    max_events_per_tenant: capacity
                },
                max_query_limit: 1
            }),
            Err(ObservabilityError::LimitExceeded)
        );
    }
}

#[test]
fn exact_pinned_toml_decoder_accepts_both_closed_backend_shapes() {
    let local: ObservabilityConfigV1 = toml::from_str(
        r#"
version = 1
max_query_limit = 32
[sink]
type = "local"
max_events_per_tenant = 128
"#,
    )
    .expect("local TOML");
    assert_eq!(
        ObservabilitySettings::try_from(local)
            .expect("settings")
            .sink,
        SinkSettings::Local {
            max_events_per_tenant: 128
        }
    );

    let otel: ObservabilityConfigV1 = toml::from_str(
        r#"
version = 1
max_query_limit = 8
[sink]
type = "open_telemetry_logs"
"#,
    )
    .expect("otel TOML");
    assert_eq!(
        ObservabilitySettings::try_from(otel)
            .expect("settings")
            .sink,
        SinkSettings::OpenTelemetryLogs
    );
    assert!(
        toml::from_str::<ObservabilityConfigV1>(
            "version=1\nmax_query_limit=8\nextra=true\n[sink]\ntype='open_telemetry_logs'"
        )
        .is_err()
    );
}
