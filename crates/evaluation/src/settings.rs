use crate::{EvaluationError, MAX_RESULTS_GLOBAL, MAX_RESULTS_PER_TENANT};
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ConfigVersion {
    V1,
}
#[derive(Clone, Copy, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
pub enum ExecutorBackendV1 {
    LocalDeterministic,
    SerdesAiEvals,
}
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum StoreConfigV1 {
    InMemory {
        max_results_per_tenant: usize,
        max_results_global: usize,
    },
}
#[derive(Clone, Debug, Deserialize, Eq, JsonSchema, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct EvaluationConfigV1 {
    pub version: ConfigVersion,
    pub executor: ExecutorBackendV1,
    pub store: StoreConfigV1,
}
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EvaluationSettings {
    executor: ExecutorBackendV1,
    max_results_per_tenant: usize,
    max_results_global: usize,
}
impl TryFrom<EvaluationConfigV1> for EvaluationSettings {
    type Error = EvaluationError;
    fn try_from(config: EvaluationConfigV1) -> Result<Self, Self::Error> {
        let StoreConfigV1::InMemory {
            max_results_per_tenant,
            max_results_global,
        } = config.store;
        if max_results_per_tenant == 0
            || max_results_global == 0
            || max_results_per_tenant > MAX_RESULTS_PER_TENANT
            || max_results_global > MAX_RESULTS_GLOBAL
            || max_results_per_tenant > max_results_global
        {
            return Err(EvaluationError::LimitExceeded);
        }
        Ok(Self {
            executor: config.executor,
            max_results_per_tenant,
            max_results_global,
        })
    }
}
impl EvaluationSettings {
    #[must_use]
    pub const fn executor(&self) -> ExecutorBackendV1 {
        self.executor
    }
    #[must_use]
    pub const fn max_results_per_tenant(&self) -> usize {
        self.max_results_per_tenant
    }
    #[must_use]
    pub const fn max_results_global(&self) -> usize {
        self.max_results_global
    }
}
