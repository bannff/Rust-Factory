#![cfg(any(feature = "local", feature = "redb"))]

use std::path::PathBuf;
use std::sync::{Arc, Barrier};
use std::thread;
use std::time::{SystemTime, UNIX_EPOCH};

use storage::{
    DeleteCondition, DeleteOutcome, ListLimit, ListRequest, Namespace, ObjectKey, ObjectStore,
    ObjectValue, PersistenceGuarantee, PutCondition, PutOutcome, StorageError, StorageLimits,
    StorageScope, TenantId,
};

struct TempPath(PathBuf);

impl TempPath {
    #[cfg_attr(not(feature = "redb"), allow(dead_code))]
    fn new() -> Self {
        static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        Self(std::env::temp_dir().join(format!(
            "storage-contract-{}-{nanos}-{nonce}.redb",
            std::process::id()
        )))
    }
}

impl Drop for TempPath {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

struct TestStore {
    name: &'static str,
    store: Arc<dyn ObjectStore>,
    _path: Option<TempPath>,
}

#[allow(clippy::vec_init_then_push, reason = "feature-gated adapter matrix")]
fn stores(limits: StorageLimits) -> Vec<TestStore> {
    let mut stores = Vec::new();
    #[cfg(feature = "local")]
    stores.push(TestStore {
        name: "local",
        store: Arc::new(storage::local::LocalObjectStore::new(limits)),
        _path: None,
    });
    #[cfg(feature = "redb")]
    {
        let path = TempPath::new();
        let store = storage::redb::RedbObjectStore::open(&path.0, limits).expect("redb opens");
        stores.push(TestStore {
            name: "redb",
            store: Arc::new(store),
            _path: Some(path),
        });
    }
    stores
}

fn limits(
    objects_per_tenant: u64,
    bytes_per_tenant: u64,
    objects_global: u64,
    bytes_global: u64,
) -> StorageLimits {
    StorageLimits::new(
        objects_per_tenant,
        bytes_per_tenant,
        objects_global,
        bytes_global,
    )
    .unwrap()
}

fn scope(tenant: &str, namespace: &str) -> StorageScope {
    StorageScope::new(
        TenantId::new(tenant).unwrap(),
        Namespace::new(namespace).unwrap(),
    )
}

fn key(bytes: impl Into<Vec<u8>>) -> ObjectKey {
    ObjectKey::new(bytes).unwrap()
}
fn value(bytes: impl Into<Vec<u8>>) -> ObjectValue {
    ObjectValue::new(bytes).unwrap()
}

fn created(outcome: PutOutcome) -> storage::ObjectVersion {
    let PutOutcome::Created { version } = outcome else {
        panic!("expected Created, got {outcome:?}")
    };
    version
}

fn replaced(outcome: PutOutcome) -> storage::ObjectVersion {
    let PutOutcome::Replaced { version } = outcome else {
        panic!("expected Replaced, got {outcome:?}")
    };
    version
}

#[test]
fn generic_suite_reports_truthful_guarantees() {
    for test in stores(limits(3, 9, 4, 10)) {
        let guarantees = test.store.guarantees();
        assert_eq!(
            guarantees.limits,
            limits(3, 9, 4, 10),
            "{} limits",
            test.name
        );
        assert!(!guarantees.shared_across_processes, "{} sharing", test.name);
        assert!(guarantees.per_operation_atomic, "{} atomic", test.name);
        assert!(guarantees.conditional_writes, "{} conditions", test.name);
        assert!(!guarantees.eviction, "{} eviction", test.name);
        #[cfg(feature = "local")]
        if test.name == "local" {
            assert_eq!(guarantees.persistence, PersistenceGuarantee::Volatile);
        }
        #[cfg(feature = "redb")]
        if test.name == "redb" {
            assert_eq!(
                guarantees.persistence,
                PersistenceGuarantee::ImmediateCommit
            );
        }
    }
}

#[test]
fn generic_suite_preserves_empty_and_maximum_values_byte_exactly() {
    for test in stores(limits(2, 2_097_152, 2, 2_097_152)) {
        let scope = scope("tenant", "objects");
        for (raw_key, raw_value) in [
            (vec![0], vec![]),
            (vec![0xff; 1_024], vec![0xa5; 1_048_576]),
        ] {
            let object_key = key(raw_key);
            let object_value = value(raw_value.clone());
            test.store
                .put(&scope, object_key.clone(), object_value, PutCondition::Any)
                .unwrap();
            assert_eq!(
                test.store
                    .get(&scope, &object_key)
                    .unwrap()
                    .unwrap()
                    .value
                    .as_bytes(),
                raw_value,
                "{} byte exactness",
                test.name
            );
        }
    }
}

#[test]
fn generic_suite_covers_every_put_and_delete_condition_outcome() {
    for test in stores(limits(5, 20, 5, 20)) {
        let scope = scope("t", "n");
        let k = key(b"k".to_vec());
        let v1 = created(
            test.store
                .put(
                    &scope,
                    k.clone(),
                    value(b"one".to_vec()),
                    PutCondition::IfAbsent,
                )
                .unwrap(),
        );
        assert_eq!(
            test.store
                .put(
                    &scope,
                    k.clone(),
                    value(b"x".to_vec()),
                    PutCondition::IfAbsent
                )
                .unwrap(),
            PutOutcome::Conflict,
            "{} if absent conflict",
            test.name
        );
        let stale = created(
            test.store
                .put(
                    &scope,
                    key(b"other".to_vec()),
                    value(vec![]),
                    PutCondition::Any,
                )
                .unwrap(),
        );
        assert_eq!(
            test.store
                .put(
                    &scope,
                    k.clone(),
                    value(b"x".to_vec()),
                    PutCondition::IfVersion(stale.clone())
                )
                .unwrap(),
            PutOutcome::Conflict,
            "{} stale put",
            test.name
        );
        let v2 = replaced(
            test.store
                .put(
                    &scope,
                    k.clone(),
                    value(b"two".to_vec()),
                    PutCondition::IfVersion(v1),
                )
                .unwrap(),
        );
        let _v3 = replaced(
            test.store
                .put(
                    &scope,
                    k.clone(),
                    value(b"three".to_vec()),
                    PutCondition::Any,
                )
                .unwrap(),
        );
        assert_eq!(
            test.store
                .delete(&scope, &k, DeleteCondition::IfVersion(v2))
                .unwrap(),
            DeleteOutcome::Conflict,
            "{} stale delete",
            test.name
        );
        assert_eq!(
            test.store.delete(&scope, &k, DeleteCondition::Any).unwrap(),
            DeleteOutcome::Deleted,
            "{} any delete",
            test.name
        );
        assert_eq!(
            test.store.delete(&scope, &k, DeleteCondition::Any).unwrap(),
            DeleteOutcome::NotFound,
            "{} absent any delete",
            test.name
        );
        assert_eq!(
            test.store
                .delete(&scope, &k, DeleteCondition::IfVersion(stale))
                .unwrap(),
            DeleteOutcome::NotFound,
            "{} absent conditional delete",
            test.name
        );
    }
}

#[test]
fn generic_suite_failed_conditions_and_not_found_consume_no_revision() {
    for test in stores(limits(10, 100, 10, 100)) {
        let baseline = stores(limits(10, 100, 10, 100))
            .into_iter()
            .find(|candidate| candidate.name == test.name)
            .unwrap();
        let scope = scope("t", "n");
        let first = created(
            test.store
                .put(&scope, key(b"a".to_vec()), value(vec![]), PutCondition::Any)
                .unwrap(),
        );
        let baseline_first = created(
            baseline
                .store
                .put(&scope, key(b"a".to_vec()), value(vec![]), PutCondition::Any)
                .unwrap(),
        );
        assert_eq!(first, baseline_first);
        assert_eq!(
            test.store
                .put(
                    &scope,
                    key(b"a".to_vec()),
                    value(vec![]),
                    PutCondition::IfAbsent
                )
                .unwrap(),
            PutOutcome::Conflict
        );
        assert_eq!(
            test.store
                .delete(&scope, &key(b"missing".to_vec()), DeleteCondition::Any)
                .unwrap(),
            DeleteOutcome::NotFound
        );
        let second = created(
            test.store
                .put(&scope, key(b"b".to_vec()), value(vec![]), PutCondition::Any)
                .unwrap(),
        );
        let baseline_second = created(
            baseline
                .store
                .put(&scope, key(b"b".to_vec()), value(vec![]), PutCondition::Any)
                .unwrap(),
        );
        assert_eq!(
            second, baseline_second,
            "{} failed operations consumed revision",
            test.name
        );
    }
}

#[test]
fn generic_suite_isolates_tenants_namespaces_conditions_lists_and_quotas() {
    for test in stores(limits(1, 4, 3, 12)) {
        let a = scope("a", "x");
        let other_namespace = scope("a", "y");
        let other_tenant = scope("b", "x");
        let k = key(b"same".to_vec());
        test.store
            .put(&a, k.clone(), value(b"aaaa".to_vec()), PutCondition::Any)
            .unwrap();
        assert_eq!(test.store.get(&other_namespace, &k).unwrap(), None);
        assert_eq!(test.store.get(&other_tenant, &k).unwrap(), None);
        assert_eq!(
            test.store
                .delete(&other_namespace, &k, DeleteCondition::Any)
                .unwrap(),
            DeleteOutcome::NotFound
        );
        assert!(
            test.store
                .list(
                    &other_tenant,
                    &ListRequest::new(None, ListLimit::new(1).unwrap())
                )
                .unwrap()
                .objects
                .is_empty()
        );
        assert!(
            matches!(
                test.store.put(
                    &other_namespace,
                    k.clone(),
                    value(vec![]),
                    PutCondition::IfAbsent
                ),
                Err(StorageError::LimitExceeded)
            ),
            "{} tenant quota crosses namespaces",
            test.name
        );
        assert!(matches!(
            test.store
                .put(
                    &other_tenant,
                    k,
                    value(b"bbbb".to_vec()),
                    PutCondition::IfAbsent
                )
                .unwrap(),
            PutOutcome::Created { .. }
        ));
    }
}

#[test]
fn generic_suite_delete_recreate_resists_aba_and_clones_read_writes() {
    for test in stores(limits(2, 10, 2, 10)) {
        let scope = scope("t", "n");
        let k = key(b"k".to_vec());
        let old = created(
            test.store
                .put(&scope, k.clone(), value(b"old".to_vec()), PutCondition::Any)
                .unwrap(),
        );
        let clone = Arc::clone(&test.store);
        assert_eq!(
            clone
                .delete(&scope, &k, DeleteCondition::IfVersion(old.clone()))
                .unwrap(),
            DeleteOutcome::Deleted
        );
        let new = created(
            clone
                .put(
                    &scope,
                    k.clone(),
                    value(b"new".to_vec()),
                    PutCondition::IfAbsent,
                )
                .unwrap(),
        );
        assert_ne!(old, new, "{} ABA version reuse", test.name);
        assert_eq!(
            test.store
                .put(
                    &scope,
                    k.clone(),
                    value(b"stale".to_vec()),
                    PutCondition::IfVersion(old)
                )
                .unwrap(),
            PutOutcome::Conflict
        );
        assert_eq!(
            test.store
                .get(&scope, &k)
                .unwrap()
                .unwrap()
                .value
                .as_bytes(),
            b"new"
        );
    }
}

#[test]
fn generic_suite_replacement_growth_shrink_and_capacity_are_delta_accounted() {
    for test in stores(limits(1, 5, 1, 5)) {
        let scope = scope("t", "n");
        let k = key(b"k".to_vec());
        test.store
            .put(
                &scope,
                k.clone(),
                value(b"12345".to_vec()),
                PutCondition::Any,
            )
            .unwrap();
        assert!(
            matches!(
                test.store
                    .put(
                        &scope,
                        k.clone(),
                        value(b"abcde".to_vec()),
                        PutCondition::Any
                    )
                    .unwrap(),
                PutOutcome::Replaced { .. }
            ),
            "{} equal replacement at capacity",
            test.name
        );
        test.store
            .put(&scope, k.clone(), value(b"1".to_vec()), PutCondition::Any)
            .unwrap();
        test.store
            .put(
                &scope,
                k.clone(),
                value(b"12345".to_vec()),
                PutCondition::Any,
            )
            .unwrap();
        assert_eq!(
            test.store.put(
                &scope,
                k.clone(),
                value(b"123456".to_vec()),
                PutCondition::Any
            ),
            Err(StorageError::LimitExceeded)
        );
        assert_eq!(
            test.store
                .get(&scope, &k)
                .unwrap()
                .unwrap()
                .value
                .as_bytes(),
            b"12345",
            "{} failed growth mutated",
            test.name
        );
    }
}

#[test]
fn generic_suite_enforces_per_tenant_and_global_count_and_byte_quotas_without_eviction() {
    for test in stores(limits(2, 4, 3, 6)) {
        let a = scope("a", "n");
        let b = scope("b", "n");
        for (scope, raw_key, raw_value) in [
            (&a, b"a".to_vec(), b"12".to_vec()),
            (&a, b"b".to_vec(), b"34".to_vec()),
            (&b, b"c".to_vec(), b"56".to_vec()),
        ] {
            test.store
                .put(scope, key(raw_key), value(raw_value), PutCondition::Any)
                .unwrap();
        }
        assert_eq!(
            test.store
                .put(&a, key(b"d".to_vec()), value(vec![]), PutCondition::Any),
            Err(StorageError::LimitExceeded)
        );
        assert_eq!(
            test.store
                .put(&b, key(b"d".to_vec()), value(vec![]), PutCondition::Any),
            Err(StorageError::LimitExceeded)
        );
        assert_eq!(
            test.store.put(
                &b,
                key(b"c".to_vec()),
                value(b"567".to_vec()),
                PutCondition::Any
            ),
            Err(StorageError::LimitExceeded)
        );
        for (scope, raw_key, expected) in [
            (&a, b"a".to_vec(), b"12".as_slice()),
            (&a, b"b".to_vec(), b"34".as_slice()),
            (&b, b"c".to_vec(), b"56".as_slice()),
        ] {
            assert_eq!(
                test.store
                    .get(scope, &key(raw_key))
                    .unwrap()
                    .unwrap()
                    .value
                    .as_bytes(),
                expected,
                "{} no eviction",
                test.name
            );
        }
        assert_eq!(
            test.store
                .delete(&a, &key(b"a".to_vec()), DeleteCondition::Any)
                .unwrap(),
            DeleteOutcome::Deleted
        );
        assert!(
            matches!(
                test.store
                    .put(
                        &b,
                        key(b"d".to_vec()),
                        value(b"1".to_vec()),
                        PutCondition::Any
                    )
                    .unwrap(),
                PutOutcome::Created { .. }
            ),
            "{} delete releases quota",
            test.name
        );
    }
}

#[test]
fn generic_suite_lists_arbitrary_keys_in_unsigned_order_with_exclusive_pages() {
    for test in stores(limits(10, 100, 10, 100)) {
        let scope = scope("t", "n");
        let raw_keys = [vec![0], vec![0, 0], vec![0, 255], vec![1], vec![255]];
        for raw in raw_keys.iter().rev() {
            test.store
                .put(&scope, key(raw.clone()), value(vec![]), PutCondition::Any)
                .unwrap();
        }
        let first = test
            .store
            .list(&scope, &ListRequest::new(None, ListLimit::new(2).unwrap()))
            .unwrap();
        assert_eq!(
            first
                .objects
                .iter()
                .map(|item| item.key.as_bytes())
                .collect::<Vec<_>>(),
            vec![raw_keys[0].as_slice(), raw_keys[1].as_slice()],
            "{} first-page lower bound",
            test.name
        );
        assert!(first.has_more);
        let second = test
            .store
            .list(
                &scope,
                &ListRequest::new(
                    Some(first.objects.last().unwrap().key.clone()),
                    ListLimit::new(2).unwrap(),
                ),
            )
            .unwrap();
        assert_eq!(
            second
                .objects
                .iter()
                .map(|item| item.key.as_bytes())
                .collect::<Vec<_>>(),
            vec![raw_keys[2].as_slice(), raw_keys[3].as_slice()]
        );
        assert!(second.has_more);
        let third = test
            .store
            .list(
                &scope,
                &ListRequest::new(
                    Some(second.objects.last().unwrap().key.clone()),
                    ListLimit::new(2).unwrap(),
                ),
            )
            .unwrap();
        assert_eq!(
            third
                .objects
                .iter()
                .map(|item| item.key.as_bytes())
                .collect::<Vec<_>>(),
            vec![raw_keys[4].as_slice()]
        );
        assert!(!third.has_more);
        let repeated = test
            .store
            .list(&scope, &ListRequest::new(None, ListLimit::new(2).unwrap()))
            .unwrap();
        assert_eq!(first, repeated, "{} quiescent pagination", test.name);
        let beyond = test
            .store
            .list(
                &scope,
                &ListRequest::new(Some(key(vec![255, 255])), ListLimit::new(2).unwrap()),
            )
            .unwrap();
        assert!(beyond.objects.is_empty());
        assert!(!beyond.has_more);
    }
}

#[test]
fn generic_suite_concurrent_cas_has_exactly_one_winner() {
    for test in stores(limits(2, 10, 2, 10)) {
        let scope = scope("t", "n");
        let k = key(b"k".to_vec());
        let initial = created(
            test.store
                .put(&scope, k.clone(), value(vec![]), PutCondition::Any)
                .unwrap(),
        );
        let barrier = Arc::new(Barrier::new(9));
        let mut workers = Vec::new();
        for byte in 0_u8..8 {
            let store = Arc::clone(&test.store);
            let scope = scope.clone();
            let k = k.clone();
            let initial = initial.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                store
                    .put(
                        &scope,
                        k,
                        value(vec![byte]),
                        PutCondition::IfVersion(initial),
                    )
                    .unwrap()
            }));
        }
        barrier.wait();
        let outcomes = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, PutOutcome::Replaced { .. }))
                .count(),
            1,
            "{} CAS winners",
            test.name
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == PutOutcome::Conflict)
                .count(),
            7,
            "{} CAS conflicts",
            test.name
        );
    }
}

#[test]
fn generic_suite_concurrent_capacity_race_never_overshoots() {
    for test in stores(limits(1, 1, 1, 1)) {
        let barrier = Arc::new(Barrier::new(9));
        let mut workers = Vec::new();
        for byte in 0_u8..8 {
            let store = Arc::clone(&test.store);
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                store.put(
                    &scope("t", "n"),
                    key(vec![byte]),
                    value(vec![byte]),
                    PutCondition::IfAbsent,
                )
            }));
        }
        barrier.wait();
        let outcomes = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            outcomes
                .iter()
                .filter(|result| matches!(result, Ok(PutOutcome::Created { .. })))
                .count(),
            1,
            "{} capacity winners",
            test.name
        );
        assert_eq!(
            test.store
                .list(
                    &scope("t", "n"),
                    &ListRequest::new(None, ListLimit::new(10).unwrap())
                )
                .unwrap()
                .objects
                .len(),
            1,
            "{} capacity overshoot",
            test.name
        );
    }
}

#[test]
fn generic_suite_concurrent_replacement_growth_and_delete_races_are_atomic() {
    for test in stores(limits(2, 1, 2, 1)) {
        let scope = scope("t", "n");
        let a = key(b"a".to_vec());
        let b = key(b"b".to_vec());
        let va = created(
            test.store
                .put(&scope, a.clone(), value(vec![]), PutCondition::Any)
                .unwrap(),
        );
        let vb = created(
            test.store
                .put(&scope, b.clone(), value(vec![]), PutCondition::Any)
                .unwrap(),
        );
        let barrier = Arc::new(Barrier::new(3));
        let mut workers = Vec::new();
        for (key, version) in [(a.clone(), va), (b.clone(), vb)] {
            let store = Arc::clone(&test.store);
            let scope = scope.clone();
            let barrier = Arc::clone(&barrier);
            workers.push(thread::spawn(move || {
                barrier.wait();
                store.put(
                    &scope,
                    key,
                    value(vec![1]),
                    PutCondition::IfVersion(version),
                )
            }));
        }
        barrier.wait();
        let growth = workers
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            growth
                .iter()
                .filter(|result| matches!(result, Ok(PutOutcome::Replaced { .. })))
                .count(),
            1,
            "{} growth winners",
            test.name
        );
        assert_eq!(
            growth
                .iter()
                .filter(|result| **result == Err(StorageError::LimitExceeded))
                .count(),
            1,
            "{} growth quota losers",
            test.name
        );

        let barrier = Arc::new(Barrier::new(3));
        let mut deletes = Vec::new();
        for _ in 0..2 {
            let store = Arc::clone(&test.store);
            let scope = scope.clone();
            let a = a.clone();
            let barrier = Arc::clone(&barrier);
            deletes.push(thread::spawn(move || {
                barrier.wait();
                store.delete(&scope, &a, DeleteCondition::Any).unwrap()
            }));
        }
        barrier.wait();
        let outcomes = deletes
            .into_iter()
            .map(|worker| worker.join().unwrap())
            .collect::<Vec<_>>();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == DeleteOutcome::Deleted)
                .count(),
            1,
            "{} delete winners",
            test.name
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| **outcome == DeleteOutcome::NotFound)
                .count(),
            1,
            "{} delete not-found",
            test.name
        );
    }
}

#[test]
fn generic_suite_maximum_page_has_exact_has_more_semantics() {
    for test in stores(limits(1_001, 1, 1_001, 1)) {
        let scope = scope("t", "n");
        for number in 0_u16..1_001 {
            test.store
                .put(
                    &scope,
                    key(number.to_be_bytes().to_vec()),
                    value(vec![]),
                    PutCondition::Any,
                )
                .unwrap();
        }
        let page = test
            .store
            .list(
                &scope,
                &ListRequest::new(None, ListLimit::new(1_000).unwrap()),
            )
            .unwrap();
        assert_eq!(
            page.objects.len(),
            1_000,
            "{} maximum page length",
            test.name
        );
        assert!(page.has_more, "{} maximum page has_more", test.name);
        let tail = test
            .store
            .list(
                &scope,
                &ListRequest::new(
                    Some(page.objects.last().unwrap().key.clone()),
                    ListLimit::new(1_000).unwrap(),
                ),
            )
            .unwrap();
        assert_eq!(tail.objects.len(), 1, "{} tail page", test.name);
        assert!(!tail.has_more, "{} exact tail has_more", test.name);
    }
}
