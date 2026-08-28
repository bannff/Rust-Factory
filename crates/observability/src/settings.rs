use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

use crate::{MAX_LOCAL_EVENTS_PER_TENANT, MAX_QUERY_LIMIT, ObservabilityError};

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(deny_unknown_fields)]
pub struct ObservabilityConfigV1 {
    pub version: u8,
    pub sink: SinkConfigV1,
    pub max_query_limit: usize,
}

#[derive(Clone, Debug, Deserialize, JsonSchema, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum SinkConfigV1 {
    Local { max_events_per_tenant: usize },
    OpenTelemetryLogs,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ObservabilitySettings {
    pub sink: SinkSettings,
    pub max_query_limit: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SinkSettings {
    Local { max_events_per_tenant: usize },
    OpenTelemetryLogs,
}

impl TryFrom<ObservabilityConfigV1> for ObservabilitySettings {
    type Error = ObservabilityError;

    fn try_from(config: ObservabilityConfigV1) -> Result<Self, Self::Error> {
        if config.version != 1 {
            return Err(ObservabilityError::InvalidConfiguration);
        }
        if config.max_query_limit == 0 || config.max_query_limit > MAX_QUERY_LIMIT {
            return Err(ObservabilityError::LimitExceeded);
        }
        let sink = match config.sink {
            SinkConfigV1::Local {
                max_events_per_tenant,
            } => {
                if max_events_per_tenant == 0 || max_events_per_tenant > MAX_LOCAL_EVENTS_PER_TENANT
                {
                    return Err(ObservabilityError::LimitExceeded);
                }
                SinkSettings::Local {
                    max_events_per_tenant,
                }
            }
            SinkConfigV1::OpenTelemetryLogs => SinkSettings::OpenTelemetryLogs,
        };
        Ok(Self {
            sink,
            max_query_limit: config.max_query_limit,
        })
    }
}
