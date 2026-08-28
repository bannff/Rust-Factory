//! Deterministic in-memory evaluation adapters.
//!
//! Deterministic process-local adapters, enabled by the `memory` feature.
//! No persistence, recovery, or cross-process durability guarantee.

#![allow(clippy::missing_errors_doc)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use crate::{
    CreateOrMatch, EvaluationError, EvaluationResultV1, EvaluationStore, LogicalEvaluationKey,
    validate_result,
};

#[derive(Clone, Default)]
pub struct InMemoryEvaluationStore {
    state: Arc<Mutex<BTreeMap<LogicalEvaluationKey, EvaluationResultV1>>>,
}
impl EvaluationStore for InMemoryEvaluationStore {
    fn create_or_match(
        &self,
        result: EvaluationResultV1,
    ) -> Result<CreateOrMatch, EvaluationError> {
        validate_result(&result)?;
        let mut state = self
            .state
            .lock()
            .map_err(|_| EvaluationError::AdapterFailure)?;
        match state.get(&result.logical_key) {
            None => {
                state.insert(result.logical_key.clone(), result.clone());
                Ok(CreateOrMatch::Created(result))
            }
            Some(existing) if existing.content_hash == result.content_hash => {
                Ok(CreateOrMatch::Existing(existing.clone()))
            }
            Some(_) => Ok(CreateOrMatch::Conflict),
        }
    }
    fn get(
        &self,
        tenant_id: &str,
        key: &LogicalEvaluationKey,
    ) -> Result<Option<EvaluationResultV1>, EvaluationError> {
        Ok((key.tenant_id == tenant_id)
            .then(|| {
                self.state
                    .lock()
                    .map_err(|_| EvaluationError::AdapterFailure)
            })
            .transpose()?
            .and_then(|state| state.get(key).cloned()))
    }
    fn list(&self, tenant_id: &str) -> Result<Vec<EvaluationResultV1>, EvaluationError> {
        Ok(self
            .state
            .lock()
            .map_err(|_| EvaluationError::AdapterFailure)?
            .values()
            .filter(|result| result.logical_key.tenant_id == tenant_id)
            .cloned()
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Barrier};
    use std::thread;

    fn key(tenant: &str) -> LogicalEvaluationKey {
        LogicalEvaluationKey {
            tenant_id: tenant.to_owned(),
            evaluator_id: "evaluator".to_owned(),
            evaluator_version: "1".to_owned(),
            criterion_digest: "a".repeat(64),
            workflow_run_id: "run".to_owned(),
            workflow_revision: 1,
        }
    }
    fn result(tenant: &str) -> EvaluationResultV1 {
        let mut result = EvaluationResultV1 {
            logical_key: key(tenant),
            evidence_digest: "b".repeat(64),
            verdict: crate::Verdict::Pass,
            findings: vec![],
            content_hash: String::new(),
        };
        result.content_hash = crate::result_digest(&result).expect("digest");
        result
    }

    #[test]
    fn store_hides_other_tenant_results_and_lists_only_own_tenant() {
        let store = InMemoryEvaluationStore::default();
        let tenant_result = result("tenant-a");
        let other_result = result("tenant-b");
        assert!(matches!(
            store.create_or_match(tenant_result.clone()),
            Ok(CreateOrMatch::Created(_))
        ));
        assert!(matches!(
            store.create_or_match(other_result),
            Ok(CreateOrMatch::Created(_))
        ));
        assert_eq!(
            store
                .get("tenant-b", &tenant_result.logical_key)
                .expect("read"),
            None
        );
        assert_eq!(store.list("tenant-a").expect("list"), vec![tenant_result]);
    }

    #[test]
    fn store_matches_identical_content_and_rejects_same_key_different_content() {
        let store = InMemoryEvaluationStore::default();
        let original = result("tenant");
        assert!(matches!(
            store.create_or_match(original.clone()),
            Ok(CreateOrMatch::Created(_))
        ));
        assert!(matches!(
            store.create_or_match(original),
            Ok(CreateOrMatch::Existing(_))
        ));
        let mut different = result("tenant");
        different.findings.push("different".to_owned());
        different.content_hash = crate::result_digest(&different).expect("digest");
        assert_eq!(
            store.create_or_match(different).expect("write"),
            CreateOrMatch::Conflict
        );
    }

    #[test]
    fn store_rejects_a_tampered_content_hash() {
        let store = InMemoryEvaluationStore::default();
        let mut tampered = result("tenant");
        tampered.content_hash = "a".repeat(64);
        assert_eq!(
            store.create_or_match(tampered),
            Err(EvaluationError::InvalidRequest)
        );
    }

    #[test]
    fn store_rejects_different_content_with_the_same_claimed_hash() {
        let store = InMemoryEvaluationStore::default();
        let original = result("tenant");
        let mut forged = original.clone();
        forged.findings.push("different".to_owned());
        assert_eq!(
            store.create_or_match(forged),
            Err(EvaluationError::InvalidRequest)
        );
    }

    #[test]
    fn concurrent_create_or_match_has_exactly_one_creator_and_no_conflicts() {
        let store = Arc::new(InMemoryEvaluationStore::default());
        let barrier = Arc::new(Barrier::new(8));
        let workers: Vec<_> = (0..8)
            .map(|_| {
                let store = Arc::clone(&store);
                let barrier = Arc::clone(&barrier);
                thread::spawn(move || {
                    barrier.wait();
                    store.create_or_match(result("tenant")).expect("write")
                })
            })
            .collect();
        let outcomes: Vec<_> = workers
            .into_iter()
            .map(|worker| worker.join().expect("thread"))
            .collect();
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, CreateOrMatch::Created(_)))
                .count(),
            1
        );
        assert_eq!(
            outcomes
                .iter()
                .filter(|outcome| matches!(outcome, CreateOrMatch::Existing(_)))
                .count(),
            7
        );
        assert_eq!(store.list("tenant").expect("list").len(), 1);
    }
}
