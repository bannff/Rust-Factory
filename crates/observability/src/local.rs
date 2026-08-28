use std::collections::{BTreeMap, VecDeque};
use std::sync::{Arc, Mutex};

use crate::{
    MAX_LOCAL_EVENTS_PER_TENANT, ObservabilityError, TelemetryEnvelopeV1, TelemetryGuarantees,
    TelemetryQueryV1, TelemetryReader, TelemetryRecordV1, TelemetrySink, TenantId, record_matches,
    validate_envelope, validate_query, validate_record,
};

struct State {
    next_sequence: u64,
    total_events: usize,
    tenants: BTreeMap<TenantId, VecDeque<TelemetryRecordV1>>,
}

#[derive(Clone)]
pub struct LocalTelemetry {
    capacity_per_tenant: usize,
    state: Arc<Mutex<State>>,
}
impl LocalTelemetry {
    pub fn new(capacity_per_tenant: usize) -> Result<Self, ObservabilityError> {
        if capacity_per_tenant == 0 || capacity_per_tenant > MAX_LOCAL_EVENTS_PER_TENANT {
            return Err(ObservabilityError::LimitExceeded);
        }
        Ok(Self {
            capacity_per_tenant,
            state: Arc::new(Mutex::new(State {
                next_sequence: 1,
                total_events: 0,
                tenants: BTreeMap::new(),
            })),
        })
    }
}
impl TelemetrySink for LocalTelemetry {
    fn emit(&self, envelope: TelemetryEnvelopeV1) -> Result<(), ObservabilityError> {
        validate_envelope(&envelope)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| ObservabilityError::AdapterFailure)?;
        let tenant_id = envelope.tenant_id.clone();
        let tenant_is_full = state
            .tenants
            .get(&tenant_id)
            .is_some_and(|records| records.len() == self.capacity_per_tenant);
        if !tenant_is_full && state.total_events == crate::MAX_LOCAL_EVENTS_TOTAL {
            return Err(ObservabilityError::LimitExceeded);
        }
        let sequence = state.next_sequence;
        state.next_sequence = state
            .next_sequence
            .checked_add(1)
            .ok_or(ObservabilityError::LimitExceeded)?;
        if !tenant_is_full {
            state.total_events += 1;
        }
        let records = state.tenants.entry(tenant_id).or_default();
        if tenant_is_full {
            records.pop_front();
        }
        records.push_back(TelemetryRecordV1 { sequence, envelope });
        Ok(())
    }

    fn guarantees(&self) -> TelemetryGuarantees {
        local_guarantees()
    }
}
impl TelemetryReader for LocalTelemetry {
    fn query(
        &self,
        tenant_id: &TenantId,
        query: &TelemetryQueryV1,
    ) -> Result<Vec<TelemetryRecordV1>, ObservabilityError> {
        validate_query(query, crate::MAX_QUERY_LIMIT)?;
        let state = self
            .state
            .lock()
            .map_err(|_| ObservabilityError::AdapterFailure)?;
        Ok(state
            .tenants
            .get(tenant_id)
            .into_iter()
            .flatten()
            .rev()
            .filter(|record| {
                validate_record(record).is_ok()
                    && record.envelope.tenant_id == *tenant_id
                    && record_matches(record, query)
            })
            .take(query.limit)
            .cloned()
            .collect())
    }

    fn guarantees(&self) -> TelemetryGuarantees {
        local_guarantees()
    }
}

const fn local_guarantees() -> TelemetryGuarantees {
    TelemetryGuarantees {
        durable_across_restart: false,
        visible_across_processes: false,
        delivery_confirmed: true,
        queryable: true,
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{EventName, EventTarget, Severity, TelemetryEventV1, Timestamp};

    fn envelope() -> TelemetryEnvelopeV1 {
        TelemetryEnvelopeV1 {
            tenant_id: TenantId::new("tenant").expect("tenant"),
            timestamp: Timestamp::from_unix_nanos(1),
            event: TelemetryEventV1::new(
                EventName::new("event").expect("name"),
                EventTarget::new("target").expect("target"),
                Severity::Info,
                "body",
                BTreeMap::new(),
            )
            .expect("event"),
        }
    }

    #[test]
    fn poisoned_state_fails_closed_for_reads_and_writes() {
        let store = LocalTelemetry::new(1).expect("store");
        let poisoner = store.clone();
        let _ = std::thread::spawn(move || {
            let _guard = poisoner.state.lock().expect("lock");
            panic!("poison state for deterministic failure test");
        })
        .join();
        assert_eq!(
            store.emit(envelope()),
            Err(ObservabilityError::AdapterFailure)
        );
        assert_eq!(
            store.query(
                &TenantId::new("tenant").expect("tenant"),
                &TelemetryQueryV1::new(1).expect("query")
            ),
            Err(ObservabilityError::AdapterFailure)
        );
    }
}
