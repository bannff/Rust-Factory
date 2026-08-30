use std::sync::Arc;

use crate::error::StorageError;
use crate::model::{
    DeleteCondition, DeleteOutcome, ListPage, ListRequest, ObjectKey, ObjectValue, PutCondition,
    PutOutcome, StorageScope, StoreGuarantees, StoredObject,
};

/// Bounded authoritative opaque-object storage.
pub trait ObjectStore: Send + Sync {
    fn get(
        &self,
        scope: &StorageScope,
        key: &ObjectKey,
    ) -> Result<Option<StoredObject>, StorageError>;

    fn put(
        &self,
        scope: &StorageScope,
        key: ObjectKey,
        value: ObjectValue,
        condition: PutCondition,
    ) -> Result<PutOutcome, StorageError>;

    fn delete(
        &self,
        scope: &StorageScope,
        key: &ObjectKey,
        condition: DeleteCondition,
    ) -> Result<DeleteOutcome, StorageError>;

    fn list(&self, scope: &StorageScope, request: &ListRequest) -> Result<ListPage, StorageError>;

    fn guarantees(&self) -> StoreGuarantees;
}

impl<T: ObjectStore + ?Sized> ObjectStore for Arc<T> {
    fn get(
        &self,
        scope: &StorageScope,
        key: &ObjectKey,
    ) -> Result<Option<StoredObject>, StorageError> {
        (**self).get(scope, key)
    }

    fn put(
        &self,
        scope: &StorageScope,
        key: ObjectKey,
        value: ObjectValue,
        condition: PutCondition,
    ) -> Result<PutOutcome, StorageError> {
        (**self).put(scope, key, value, condition)
    }

    fn delete(
        &self,
        scope: &StorageScope,
        key: &ObjectKey,
        condition: DeleteCondition,
    ) -> Result<DeleteOutcome, StorageError> {
        (**self).delete(scope, key, condition)
    }

    fn list(&self, scope: &StorageScope, request: &ListRequest) -> Result<ListPage, StorageError> {
        (**self).list(scope, request)
    }

    fn guarantees(&self) -> StoreGuarantees {
        (**self).guarantees()
    }
}
