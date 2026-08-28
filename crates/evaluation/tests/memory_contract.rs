#![cfg(feature = "memory")]
//! Bounded immutable in-memory store contract.

use std::sync::{Arc, Barrier};

use evaluation::memory::InMemoryEvaluationStore;
use evaluation::{
    CreateOrMatch, EvaluationError, EvaluationResultV1, EvaluationStore, LogicalEvaluationKey,
    StoreGuaranteesV1, Verdict,
};

fn result(tenant: &str, revision: u64) -> EvaluationResultV1 {
    let mut result = EvaluationResultV1 {
        logical_key: LogicalEvaluationKey {
            tenant_id: tenant.into(),
            evaluator_id: "evaluator".into(),
            evaluator_version: "1".into(),
            criterion_digest: "a".repeat(64),
            workflow_run_id: "run".into(),
            workflow_revision: revision,
        },
        evidence_digest: "b".repeat(64),
        verdict: Verdict::Pass,
        findings: vec![],
        content_hash: String::new(),
    };
    result.content_hash = evaluation::result_digest(&result).expect("hash");
    result
}
fn conflicting(mut original: EvaluationResultV1) -> EvaluationResultV1 {
    original.verdict = Verdict::Fail;
    original.findings = vec!["criterion_1_failed".into()];
    original.content_hash = evaluation::result_digest(&original).expect("hash");
    original
}

#[test]
fn constructor_capacities_and_guarantees_are_exact_and_truthful() {
    for capacities in [
        (0, 1),
        (1, 0),
        (2, 1),
        (
            evaluation::MAX_RESULTS_PER_TENANT + 1,
            evaluation::MAX_RESULTS_GLOBAL,
        ),
        (1, evaluation::MAX_RESULTS_GLOBAL + 1),
    ] {
        assert!(
            matches!(
                InMemoryEvaluationStore::with_capacities(capacities.0, capacities.1),
                Err(EvaluationError::LimitExceeded)
            ),
            "{capacities:?}"
        );
    }
    let store = InMemoryEvaluationStore::with_capacities(2, 3).expect("store");
    assert_eq!(
        store.guarantees(),
        StoreGuaranteesV1 {
            durable_across_restart: false,
            visible_across_processes: false,
            crash_atomic: false,
            evicts_on_capacity: false,
            max_results_per_tenant: 2,
            max_results_global: 3,
        }
    );
}

#[test]
fn per_tenant_and_global_limits_refuse_growth_without_eviction_or_trace() {
    let store = InMemoryEvaluationStore::with_capacities(2, 3).expect("store");
    let a1 = result("tenant-a", 1);
    let a2 = result("tenant-a", 2);
    let b1 = result("tenant-b", 1);
    for item in [a1.clone(), a2.clone(), b1.clone()] {
        assert!(matches!(
            store.create_or_match(item),
            Ok(CreateOrMatch::Created(_))
        ));
    }
    assert_eq!(
        store.create_or_match(result("tenant-a", 3)),
        Err(EvaluationError::LimitExceeded)
    );
    assert_eq!(
        store.create_or_match(result("tenant-c", 1)),
        Err(EvaluationError::LimitExceeded)
    );
    assert_eq!(store.list("tenant-a").expect("list"), vec![a1, a2]);
    assert_eq!(store.list("tenant-b").expect("list"), vec![b1]);
    assert!(store.list("tenant-c").expect("list").is_empty());
}

#[test]
fn existing_match_and_conflict_are_resolved_before_full_capacity() {
    let store = InMemoryEvaluationStore::with_capacities(1, 1).expect("store");
    let original = result("tenant", 1);
    assert!(matches!(
        store.create_or_match(original.clone()),
        Ok(CreateOrMatch::Created(_))
    ));
    assert_eq!(
        store.create_or_match(original.clone()),
        Ok(CreateOrMatch::Existing(original.clone()))
    );
    assert_eq!(
        store.create_or_match(conflicting(original)),
        Ok(CreateOrMatch::Conflict)
    );
}

#[test]
fn list_is_deterministic_and_cross_tenant_get_is_indistinguishable_from_absence() {
    let store = InMemoryEvaluationStore::with_capacities(3, 4).expect("store");
    for revision in [3, 1, 2] {
        store
            .create_or_match(result("tenant", revision))
            .expect("insert");
    }
    store.create_or_match(result("other", 1)).expect("other");
    assert_eq!(
        store
            .list("tenant")
            .expect("list")
            .iter()
            .map(|r| r.logical_key.workflow_revision)
            .collect::<Vec<_>>(),
        [1, 2, 3]
    );
    let owned = result("tenant", 1);
    assert_eq!(store.get("other", &owned.logical_key), Ok(None));
    assert_eq!(
        store.get("tenant", &result("other", 1).logical_key),
        Ok(None)
    );
}

#[test]
fn ingress_rejects_forged_hashes_and_failure_leaves_no_trace() {
    let store = InMemoryEvaluationStore::with_capacities(1, 1).expect("store");
    let mut forged_content = result("tenant", 1);
    forged_content.content_hash = "c".repeat(64);
    assert_eq!(
        store.create_or_match(forged_content),
        Err(EvaluationError::InvalidRequest)
    );
    let mut forged_evidence = result("tenant", 1);
    forged_evidence.evidence_digest = "NOT-A-HASH".into();
    forged_evidence.content_hash = evaluation::result_digest(&forged_evidence)
        .expect_err("bad evidence must not hash")
        .to_string();
    assert!(store.create_or_match(forged_evidence).is_err());
    assert!(store.list("tenant").expect("list").is_empty());
    assert!(
        matches!(
            store.create_or_match(result("tenant", 1)),
            Ok(CreateOrMatch::Created(_))
        ),
        "failed ingress must consume no capacity"
    );
}

#[test]
fn concurrent_distinct_writers_cannot_overrun_capacity() {
    let store = Arc::new(InMemoryEvaluationStore::with_capacities(1, 1).expect("store"));
    let barrier = Arc::new(Barrier::new(2));
    let workers = [1, 2].map(|revision| {
        let store = Arc::clone(&store);
        let barrier = Arc::clone(&barrier);
        std::thread::spawn(move || {
            barrier.wait();
            store.create_or_match(result("tenant", revision))
        })
    });
    let outcomes = workers.map(|worker| worker.join().expect("join"));
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| matches!(o, Ok(CreateOrMatch::Created(_))))
            .count(),
        1
    );
    assert_eq!(
        outcomes
            .iter()
            .filter(|o| **o == Err(EvaluationError::LimitExceeded))
            .count(),
        1
    );
    assert_eq!(store.list("tenant").expect("list").len(), 1);
}

#[test]
fn store_is_object_safe_behind_arc_dyn() {
    let store: Arc<dyn EvaluationStore> =
        Arc::new(InMemoryEvaluationStore::with_capacities(1, 1).expect("store"));
    assert!(matches!(
        store.create_or_match(result("tenant", 1)),
        Ok(CreateOrMatch::Created(_))
    ));
    assert_eq!(store.list("tenant").expect("list").len(), 1);
}
