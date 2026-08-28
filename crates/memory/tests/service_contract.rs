//! `MemoryService` behaviour: tenant scoping, provenance stamping, deployment
//! ceilings, and defence in depth against a misbehaving adapter.
//!
//! Gated on `local` because these need a working store. The service's own rules
//! are what is under test; the store is scenery, except where a deliberately
//! hostile store is the point.

#![cfg(feature = "local")]

use std::collections::BTreeMap;

use memory::local::{FixedClock, InProcessStore, SystemClock};
use memory::{
    Clock, MAX_QUERY_LIMIT, MemoryContext, MemoryError, MemoryKind, MemoryQuery, MemoryRecord,
    MemoryService, MemoryStore, Namespace, RecordKey, RememberRequest, RunId, StoreGuarantees,
    TenantId, Timestamp, WriteOutcome,
};

const NOW: u64 = 1_700_000_000_000_000;

fn service() -> MemoryService<InProcessStore, FixedClock> {
    MemoryService::new(
        InProcessStore::new(),
        FixedClock::new(Timestamp::from_micros(NOW)),
    )
}

fn context(tenant: &str) -> MemoryContext {
    MemoryContext::new(TenantId::new(tenant).expect("valid tenant"))
}

fn space(name: &str) -> Namespace {
    Namespace::new(name).expect("valid namespace")
}

fn key(name: &str) -> RecordKey {
    RecordKey::new(name).expect("valid key")
}

fn remember(namespace: &str, key: &str, content: &str) -> RememberRequest {
    RememberRequest {
        namespace: space(namespace),
        key: RecordKey::new(key).expect("valid key"),
        kind: MemoryKind::Factual,
        content: content.to_owned(),
        tags: Vec::new(),
        metadata: BTreeMap::new(),
        run_id: Some(RunId::new("run-1").expect("valid run id")),
    }
}

// ------------------------------------------------------- provenance and scoping

#[test]
fn the_service_stamps_provenance_from_its_clock() {
    let service = service();
    let context = context("acme");
    service
        .remember(&context, remember("notes", "stamped", "content"))
        .expect("write succeeds");

    let found = service
        .recall(&context, &space("notes"), &key("stamped"))
        .expect("recall succeeds")
        .expect("present");
    let provenance = found.provenance.expect("provenance was stamped");
    assert_eq!(
        provenance.recorded_at.as_micros(),
        NOW,
        "recorded time comes from the injected clock, not the caller"
    );
    assert_eq!(provenance.run_id.as_str(), "run-1");
}

#[test]
fn a_request_without_a_run_carries_no_provenance() {
    let service = service();
    let context = context("acme");
    let mut request = remember("notes", "unattributed", "content");
    request.run_id = None;
    service.remember(&context, request).expect("write succeeds");

    let found = service
        .recall(&context, &space("notes"), &key("unattributed"))
        .expect("recall succeeds")
        .expect("present");
    assert!(
        found.provenance.is_none(),
        "provenance is not invented when no run was named"
    );
}

#[test]
fn the_record_is_scoped_to_the_context_tenant() {
    let service = service();
    service
        .remember(&context("tenant-a"), remember("notes", "owned", "content"))
        .expect("write succeeds");

    let found = service
        .recall(&context("tenant-a"), &space("notes"), &key("owned"))
        .expect("recall succeeds")
        .expect("present");
    assert_eq!(
        found.tenant_id.as_str(),
        "tenant-a",
        "the tenant comes from context, not from the request"
    );

    // A caller cannot name a tenant in a request, so the only way to attempt a
    // cross-tenant read is with a different context, which sees nothing.
    assert!(
        service
            .recall(&context("tenant-b"), &space("notes"), &key("owned"))
            .expect("recall succeeds")
            .is_none(),
        "another tenant's context must observe absence"
    );
}

#[test]
fn rewriting_a_key_replaces_the_record() {
    let service = service();
    let context = context("acme");
    assert_eq!(
        service
            .remember(&context, remember("notes", "key", "first"))
            .expect("write"),
        WriteOutcome::Created
    );
    assert_eq!(
        service
            .remember(&context, remember("notes", "key", "second"))
            .expect("write"),
        WriteOutcome::Replaced
    );
    let all = service
        .search(
            &context,
            &space("notes"),
            &MemoryQuery::all(8).expect("valid"),
        )
        .expect("search");
    assert_eq!(all.len(), 1, "a replace leaves one record");
    assert_eq!(all[0].content, "second");
}

#[test]
fn forget_reports_whether_a_record_existed() {
    let service = service();
    let context = context("acme");
    service
        .remember(&context, remember("notes", "doomed", "content"))
        .expect("write");
    assert!(
        service
            .forget(&context, &space("notes"), &key("doomed"))
            .expect("forget")
    );
    assert!(
        !service
            .forget(&context, &space("notes"), &key("doomed"))
            .expect("forget"),
        "forgetting twice is not an error"
    );
}

#[test]
fn the_service_rejects_invalid_input_before_it_reaches_a_store() {
    let service = service();
    let mut request = remember("notes", "key", "");
    assert_eq!(
        service.remember(&context("acme"), request.clone()),
        Err(MemoryError::InvalidRecord)
    );

    request.content = "content".to_owned();
    request.tags = vec!["Not A Tag".to_owned()];
    assert_eq!(
        service.remember(&context("acme"), request),
        Err(MemoryError::InvalidRecord)
    );

    let mut query = MemoryQuery::all(8).expect("valid");
    query.term = Some(String::new());
    assert_eq!(
        service.search(&context("acme"), &space("notes"), &query),
        Err(MemoryError::InvalidQuery)
    );
}

#[test]
fn the_service_reports_the_backend_guarantees_without_overstating_them() {
    let guarantees = service().guarantees();
    assert!(!guarantees.durable_across_restart);
    assert!(!guarantees.visible_across_processes);
    assert!(!guarantees.crash_atomic);
}

// -------------------------------------------------------- deployment ceilings

#[test]
fn a_deployment_may_narrow_the_result_ceiling_but_not_widen_it() {
    for refused in [0, MAX_QUERY_LIMIT + 1] {
        assert_eq!(
            MemoryService::with_result_ceiling(
                InProcessStore::new(),
                FixedClock::new(Timestamp::from_micros(NOW)),
                refused,
            )
            .err()
            .expect("refused"),
            MemoryError::LimitExceeded,
            "a ceiling of {refused} is not a narrowing"
        );
    }

    let service = MemoryService::with_result_ceiling(
        InProcessStore::new(),
        FixedClock::new(Timestamp::from_micros(NOW)),
        4,
    )
    .expect("valid ceiling");
    assert_eq!(service.result_ceiling(), 4);

    assert!(
        service
            .search(
                &context("acme"),
                &space("notes"),
                &MemoryQuery::all(4).expect("valid")
            )
            .is_ok(),
        "a query at the ceiling is allowed"
    );
    assert_eq!(
        service.search(
            &context("acme"),
            &space("notes"),
            &MemoryQuery::all(5).expect("valid")
        ),
        Err(MemoryError::LimitExceeded),
        "a query above the ceiling is refused rather than silently truncated"
    );
}

#[test]
fn the_default_ceiling_is_the_cores_own() {
    assert_eq!(service().result_ceiling(), MAX_QUERY_LIMIT);
}

// ------------------------------------------------------------ defence in depth

/// A store that returns whatever the test tells it to, so the service's own
/// guarantees can be tested independently of any adapter being correct.
struct ScriptedStore {
    outcome: Result<Vec<MemoryRecord>, MemoryError>,
}

impl MemoryStore for ScriptedStore {
    fn put(&self, _record: MemoryRecord) -> Result<WriteOutcome, MemoryError> {
        match &self.outcome {
            Ok(_) => Ok(WriteOutcome::Created),
            Err(error) => Err(*error),
        }
    }

    fn get(
        &self,
        _tenant_id: &TenantId,
        _namespace: &Namespace,
        _key: &RecordKey,
    ) -> Result<Option<MemoryRecord>, MemoryError> {
        self.outcome
            .as_ref()
            .map(|records| records.first().cloned())
            .map_err(|error| *error)
    }

    fn query(
        &self,
        _tenant_id: &TenantId,
        _namespace: &Namespace,
        _query: &MemoryQuery,
    ) -> Result<Vec<MemoryRecord>, MemoryError> {
        self.outcome.clone()
    }

    fn delete(
        &self,
        _tenant_id: &TenantId,
        _namespace: &Namespace,
        _key: &RecordKey,
    ) -> Result<bool, MemoryError> {
        match &self.outcome {
            Ok(records) => Ok(!records.is_empty()),
            Err(error) => Err(*error),
        }
    }

    fn guarantees(&self) -> StoreGuarantees {
        StoreGuarantees::in_process()
    }
}

fn scripted(
    outcome: Result<Vec<MemoryRecord>, MemoryError>,
) -> MemoryService<ScriptedStore, FixedClock> {
    MemoryService::new(
        ScriptedStore { outcome },
        FixedClock::new(Timestamp::from_micros(NOW)),
    )
}

fn record_at(tenant: &str, namespace: &str, key_name: &str) -> MemoryRecord {
    MemoryRecord {
        tenant_id: TenantId::new(tenant).expect("valid"),
        namespace: space(namespace),
        key: key(key_name),
        kind: MemoryKind::Factual,
        content: "content".to_owned(),
        tags: Vec::new(),
        metadata: BTreeMap::new(),
        provenance: None,
    }
}

#[test]
fn an_adapter_failure_reaches_the_caller_from_every_operation() {
    // Before this there was no failing-store fixture at all, so no service error
    // path was exercised anywhere.
    let service = scripted(Err(MemoryError::AdapterFailure));
    let context = context("acme");

    assert_eq!(
        service.remember(&context, remember("notes", "key", "content")),
        Err(MemoryError::AdapterFailure)
    );
    assert_eq!(
        service.recall(&context, &space("notes"), &key("key")),
        Err(MemoryError::AdapterFailure)
    );
    assert_eq!(
        service.search(
            &context,
            &space("notes"),
            &MemoryQuery::all(8).expect("valid")
        ),
        Err(MemoryError::AdapterFailure)
    );
    assert_eq!(
        service.forget(&context, &space("notes"), &key("key")),
        Err(MemoryError::AdapterFailure)
    );
}

#[test]
fn the_service_caps_a_result_even_if_an_adapter_over_returns() {
    // A hostile or buggy adapter that ignores the limit must not make the service
    // exceed its own contract.
    let over_returned = (0..10)
        .map(|index| record_at("acme", "notes", &format!("key-{index}")))
        .collect();
    let found = scripted(Ok(over_returned))
        .search(
            &context("acme"),
            &space("notes"),
            &MemoryQuery::all(3).expect("valid"),
        )
        .expect("search");
    assert_eq!(found.len(), 3, "the service enforces the limit itself");
}

#[test]
fn the_service_drops_a_foreign_record_an_adapter_wrongly_returned() {
    // The port already forbids this, but a service that trusted the adapter would
    // turn one adapter bug into a tenant leak.
    let service = scripted(Ok(vec![record_at("other-tenant", "notes", "leaked")]));
    assert!(
        service
            .recall(&context("acme"), &space("notes"), &key("leaked"))
            .expect("recall")
            .is_none(),
        "the service must not pass through a foreign record"
    );
    assert!(
        service
            .search(
                &context("acme"),
                &space("notes"),
                &MemoryQuery::all(8).expect("valid")
            )
            .expect("search")
            .is_empty(),
        "the service must filter a foreign record out of a search"
    );
}

#[test]
fn the_service_drops_a_record_from_the_wrong_namespace_or_key() {
    // Namespace separation is a contract clause in its own right, so a same-tenant
    // adapter bug that crossed namespaces must not pass through either.
    let wrong_namespace = scripted(Ok(vec![record_at("acme", "other-namespace", "asked-for")]));
    assert!(
        wrong_namespace
            .recall(&context("acme"), &space("notes"), &key("asked-for"))
            .expect("recall")
            .is_none(),
        "a record from another namespace must not be returned"
    );
    assert!(
        wrong_namespace
            .search(
                &context("acme"),
                &space("notes"),
                &MemoryQuery::all(8).expect("valid")
            )
            .expect("search")
            .is_empty(),
        "a search must not return a record from another namespace"
    );

    let wrong_key = scripted(Ok(vec![record_at("acme", "notes", "other-key")]));
    assert!(
        wrong_key
            .recall(&context("acme"), &space("notes"), &key("asked-for"))
            .expect("recall")
            .is_none(),
        "a record under another key must not answer this read"
    );
    // A query legitimately spans keys, so the same record is fine there.
    assert_eq!(
        wrong_key
            .search(
                &context("acme"),
                &space("notes"),
                &MemoryQuery::all(8).expect("valid")
            )
            .expect("search")
            .len(),
        1,
        "a query spans keys by design, so the key is not checked"
    );
}

// ----------------------------------------------------------------------- clocks

#[test]
fn a_fixed_clock_is_deterministic_and_the_system_clock_is_real() {
    let fixed = FixedClock::new(Timestamp::from_micros(4_242));
    assert_eq!(fixed.now().as_micros(), 4_242);
    assert_eq!(fixed.now(), fixed.now(), "a fixed clock must not advance");

    // Not asserting a specific value, only that it is a real clock rather than a
    // stub that always returns zero.
    assert!(
        SystemClock.now().as_micros() > 1_600_000_000_000_000,
        "the system clock must report a plausible current time"
    );
}
