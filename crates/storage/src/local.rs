use std::collections::BTreeMap;
use std::ops::Bound::{Excluded, Included, Unbounded};
use std::sync::{Arc, Mutex};

use crate::{
    DeleteCondition, DeleteOutcome, ListPage, ListRequest, ObjectKey, ObjectMetadata, ObjectStore,
    ObjectValue, ObjectVersion, PersistenceGuarantee, PutCondition, PutOutcome, StorageError,
    StorageLimits, StorageScope, StoreGuarantees, StoredObject, TenantId,
};

type Identity = (TenantId, crate::Namespace, ObjectKey);

#[derive(Clone)]
struct Entry {
    version: ObjectVersion,
    value: ObjectValue,
}

#[derive(Clone, Copy, Default)]
struct Counters {
    objects: u64,
    value_bytes: u64,
}

#[derive(Default)]
struct State {
    objects: BTreeMap<Identity, Entry>,
    revision: u64,
    tenants: BTreeMap<TenantId, Counters>,
    global: Counters,
}

/// Deterministic process-local reference adapter.
#[derive(Clone)]
pub struct LocalObjectStore {
    state: Arc<Mutex<State>>,
    limits: StorageLimits,
}

impl LocalObjectStore {
    #[must_use]
    pub fn new(limits: StorageLimits) -> Self {
        Self {
            state: Arc::new(Mutex::new(State::default())),
            limits,
        }
    }

    #[cfg(test)]
    #[allow(
        dead_code,
        reason = "reserved for QA revision-exhaustion injection coverage"
    )]
    pub(crate) fn set_revision_for_test(&self, revision: u64) {
        self.state
            .lock()
            .expect("test lock must be healthy")
            .revision = revision;
    }

    fn identity(scope: &StorageScope, key: ObjectKey) -> Identity {
        (scope.tenant_id.clone(), scope.namespace.clone(), key)
    }

    fn checked_put_counters(
        &self,
        tenant: Counters,
        global: Counters,
        old_bytes: Option<u64>,
        new_bytes: u64,
    ) -> Result<(Counters, Counters), StorageError> {
        let (tenant_objects, global_objects) = if old_bytes.is_some() {
            (tenant.objects, global.objects)
        } else {
            (
                tenant
                    .objects
                    .checked_add(1)
                    .ok_or(StorageError::LimitExceeded)?,
                global
                    .objects
                    .checked_add(1)
                    .ok_or(StorageError::LimitExceeded)?,
            )
        };
        let old_bytes = old_bytes.unwrap_or(0);
        let tenant_bytes = tenant
            .value_bytes
            .checked_sub(old_bytes)
            .ok_or(StorageError::OperationFailed)?
            .checked_add(new_bytes)
            .ok_or(StorageError::LimitExceeded)?;
        let global_bytes = global
            .value_bytes
            .checked_sub(old_bytes)
            .ok_or(StorageError::OperationFailed)?
            .checked_add(new_bytes)
            .ok_or(StorageError::LimitExceeded)?;

        if tenant_objects > self.limits.max_objects_per_tenant()
            || tenant_bytes > self.limits.max_value_bytes_per_tenant()
            || global_objects > self.limits.max_objects_global()
            || global_bytes > self.limits.max_value_bytes_global()
        {
            return Err(StorageError::LimitExceeded);
        }
        Ok((
            Counters {
                objects: tenant_objects,
                value_bytes: tenant_bytes,
            },
            Counters {
                objects: global_objects,
                value_bytes: global_bytes,
            },
        ))
    }
}

impl ObjectStore for LocalObjectStore {
    fn get(
        &self,
        scope: &StorageScope,
        key: &ObjectKey,
    ) -> Result<Option<StoredObject>, StorageError> {
        let state = self
            .state
            .lock()
            .map_err(|_| StorageError::OperationFailed)?;
        Ok(state
            .objects
            .get(&Self::identity(scope, key.clone()))
            .map(|entry| StoredObject {
                version: entry.version.clone(),
                value: entry.value.clone(),
            }))
    }

    fn put(
        &self,
        scope: &StorageScope,
        key: ObjectKey,
        value: ObjectValue,
        condition: PutCondition,
    ) -> Result<PutOutcome, StorageError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StorageError::OperationFailed)?;
        let identity = Self::identity(scope, key);
        let current = state.objects.get(&identity);
        let condition_matches = match (&condition, current) {
            (PutCondition::Any, _) | (PutCondition::IfAbsent, None) => true,
            (PutCondition::IfVersion(expected), Some(entry)) => expected == &entry.version,
            (PutCondition::IfAbsent | PutCondition::IfVersion(_), _) => false,
        };
        if !condition_matches {
            return Ok(PutOutcome::Conflict);
        }

        let revision = state
            .revision
            .checked_add(1)
            .ok_or(StorageError::RevisionExhausted)?;
        let version = ObjectVersion::from_revision(revision)?;
        let old_bytes = current
            .map(|entry| u64::try_from(entry.value.as_bytes().len()))
            .transpose()
            .map_err(|_| StorageError::OperationFailed)?;
        let new_bytes =
            u64::try_from(value.as_bytes().len()).map_err(|_| StorageError::OperationFailed)?;
        let tenant = state
            .tenants
            .get(&scope.tenant_id)
            .copied()
            .unwrap_or_default();
        let (tenant, global) =
            self.checked_put_counters(tenant, state.global, old_bytes, new_bytes)?;
        let created = current.is_none();

        state.objects.insert(
            identity,
            Entry {
                version: version.clone(),
                value,
            },
        );
        state.tenants.insert(scope.tenant_id.clone(), tenant);
        state.global = global;
        state.revision = revision;

        Ok(if created {
            PutOutcome::Created { version }
        } else {
            PutOutcome::Replaced { version }
        })
    }

    fn delete(
        &self,
        scope: &StorageScope,
        key: &ObjectKey,
        condition: DeleteCondition,
    ) -> Result<DeleteOutcome, StorageError> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| StorageError::OperationFailed)?;
        let identity = Self::identity(scope, key.clone());
        let Some(current) = state.objects.get(&identity) else {
            return Ok(DeleteOutcome::NotFound);
        };
        if let DeleteCondition::IfVersion(expected) = &condition
            && expected != &current.version
        {
            return Ok(DeleteOutcome::Conflict);
        }

        let old_bytes = u64::try_from(current.value.as_bytes().len())
            .map_err(|_| StorageError::OperationFailed)?;
        let held = state
            .tenants
            .get(&scope.tenant_id)
            .copied()
            .ok_or(StorageError::OperationFailed)?;
        let tenant = Counters {
            objects: held
                .objects
                .checked_sub(1)
                .ok_or(StorageError::OperationFailed)?,
            value_bytes: held
                .value_bytes
                .checked_sub(old_bytes)
                .ok_or(StorageError::OperationFailed)?,
        };
        let global = Counters {
            objects: state
                .global
                .objects
                .checked_sub(1)
                .ok_or(StorageError::OperationFailed)?,
            value_bytes: state
                .global
                .value_bytes
                .checked_sub(old_bytes)
                .ok_or(StorageError::OperationFailed)?,
        };

        if tenant.objects == 0 && tenant.value_bytes != 0 {
            return Err(StorageError::OperationFailed);
        }
        state.objects.remove(&identity);
        if tenant.objects == 0 {
            state.tenants.remove(&scope.tenant_id);
        } else {
            state.tenants.insert(scope.tenant_id.clone(), tenant);
        }
        state.global = global;
        Ok(DeleteOutcome::Deleted)
    }

    fn list(&self, scope: &StorageScope, request: &ListRequest) -> Result<ListPage, StorageError> {
        let state = self
            .state
            .lock()
            .map_err(|_| StorageError::OperationFailed)?;
        let limit = request.limit.as_usize()?;
        let lower = request.after_key.as_ref().map_or_else(
            || Included(Self::identity(scope, ObjectKey::minimum())),
            |key| Excluded(Self::identity(scope, key.clone())),
        );
        let mut objects = Vec::with_capacity(limit);
        let mut has_more = false;
        for ((tenant, namespace, key), entry) in state.objects.range((lower, Unbounded)) {
            if tenant != &scope.tenant_id || namespace != &scope.namespace {
                break;
            }
            if objects.len() == limit {
                has_more = true;
                break;
            }
            objects.push(ObjectMetadata {
                key: key.clone(),
                version: entry.version.clone(),
                size_bytes: u32::try_from(entry.value.as_bytes().len())
                    .map_err(|_| StorageError::OperationFailed)?,
            });
        }
        Ok(ListPage { objects, has_more })
    }

    fn guarantees(&self) -> StoreGuarantees {
        StoreGuarantees {
            persistence: PersistenceGuarantee::Volatile,
            shared_across_processes: false,
            per_operation_atomic: true,
            conditional_writes: true,
            eviction: false,
            limits: self.limits,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Namespace, ObjectKey, ObjectValue, PutCondition, StorageScope};

    #[test]
    fn revision_exhaustion_is_condition_first_and_leaves_state_unchanged() {
        let limits = StorageLimits::new(2, 2, 2, 2).expect("valid limits");
        let store = LocalObjectStore::new(limits);
        let scope = StorageScope::new(TenantId::new("t").unwrap(), Namespace::new("n").unwrap());
        let key = ObjectKey::new(b"k".to_vec()).unwrap();
        store
            .put(
                &scope,
                key.clone(),
                ObjectValue::new(Vec::new()).unwrap(),
                PutCondition::Any,
            )
            .unwrap();
        store.set_revision_for_test(u64::MAX);

        assert_eq!(
            store.put(
                &scope,
                key.clone(),
                ObjectValue::new(vec![1]).unwrap(),
                PutCondition::IfAbsent,
            ),
            Ok(PutOutcome::Conflict)
        );
        assert_eq!(
            store.put(
                &scope,
                key.clone(),
                ObjectValue::new(vec![1]).unwrap(),
                PutCondition::Any,
            ),
            Err(StorageError::RevisionExhausted)
        );
        assert!(
            store
                .get(&scope, &key)
                .unwrap()
                .unwrap()
                .value
                .as_bytes()
                .is_empty()
        );
    }
}
