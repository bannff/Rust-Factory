use std::collections::BTreeMap;
use std::fs::OpenOptions;
use std::io::ErrorKind;
use std::ops::Bound::Excluded;
use std::path::Path;
use std::sync::Arc;

use redb::{
    Database, DatabaseError, Durability, ReadableTable, TableDefinition, TableError, TableHandle,
};

use crate::{
    DeleteCondition, DeleteOutcome, ListPage, ListRequest, MAX_NAMESPACE_BYTES,
    MAX_OBJECT_KEY_BYTES, MAX_OBJECT_VALUE_BYTES, MAX_TENANT_ID_BYTES, Namespace, ObjectKey,
    ObjectMetadata, ObjectStore, ObjectValue, ObjectVersion, PersistenceGuarantee, PutCondition,
    PutOutcome, StorageError, StorageLimits, StorageScope, StoreGuarantees, StoredObject, TenantId,
};

const OBJECTS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("storage_objects_v1");
const TENANT_QUOTAS: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("storage_tenant_quotas_v1");
const METADATA: TableDefinition<'static, &'static [u8], &'static [u8]> =
    TableDefinition::new("storage_metadata_v1");
const SCHEMA: u8 = 1;
const META_SCHEMA: &[u8] = &[0x01];
const META_REVISION: &[u8] = &[0x02];
const META_GLOBAL: &[u8] = &[0x03];

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct Counters {
    objects: u64,
    value_bytes: u64,
}

/// Exclusive-handle redb adapter with immediate durable commits.
#[derive(Clone)]
pub struct RedbObjectStore {
    database: Arc<Database>,
    limits: StorageLimits,
}

impl RedbObjectStore {
    /// Opens or creates the trusted database path and validates Storage state.
    pub fn open(path: &Path, limits: StorageLimits) -> Result<Self, StorageError> {
        let fresh = match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(reservation) => {
                drop(reservation);
                true
            }
            Err(error) if error.kind() == ErrorKind::AlreadyExists => false,
            Err(_) => return Err(StorageError::OperationFailed),
        };
        let database = if fresh {
            Database::create(path)
        } else {
            Database::open(path)
        }
        .map_err(map_database_error)?;
        let store = Self {
            database: Arc::new(database),
            limits,
        };
        match store.schema_state()? {
            SchemaState::Absent if fresh => store.initialize()?,
            SchemaState::Absent => return Err(StorageError::CorruptStore),
            SchemaState::Present => store.validate_integrity()?,
        }
        Ok(store)
    }

    fn schema_state(&self) -> Result<SchemaState, StorageError> {
        let transaction = self.database.begin_read().map_err(map_transaction_error)?;
        let mut table_count = 0;
        let mut has_objects = false;
        let mut has_tenant_quotas = false;
        let mut has_metadata = false;

        for table in transaction.list_tables().map_err(map_storage_error)? {
            table_count += 1;
            if table_count > 3 {
                return Err(StorageError::CorruptStore);
            }

            let seen = match table.name() {
                name if name == OBJECTS.name() => &mut has_objects,
                name if name == TENANT_QUOTAS.name() => &mut has_tenant_quotas,
                name if name == METADATA.name() => &mut has_metadata,
                _ => return Err(StorageError::CorruptStore),
            };
            if *seen {
                return Err(StorageError::CorruptStore);
            }
            *seen = true;
        }

        if transaction
            .list_multimap_tables()
            .map_err(map_storage_error)?
            .next()
            .is_some()
        {
            return Err(StorageError::CorruptStore);
        }
        if table_count == 0 {
            return Ok(SchemaState::Absent);
        }
        if table_count == 3 && has_objects && has_tenant_quotas && has_metadata {
            return Ok(SchemaState::Present);
        }
        Err(StorageError::CorruptStore)
    }

    fn initialize(&self) -> Result<(), StorageError> {
        let mut transaction = self.database.begin_write().map_err(map_transaction_error)?;
        transaction.set_durability(Durability::Immediate);
        {
            transaction.open_table(OBJECTS).map_err(map_table_error)?;
            transaction
                .open_table(TENANT_QUOTAS)
                .map_err(map_table_error)?;
            let mut metadata = transaction.open_table(METADATA).map_err(map_table_error)?;
            metadata
                .insert(META_SCHEMA, &[SCHEMA][..])
                .map_err(map_storage_error)?;
            metadata
                .insert(META_REVISION, &encode_revision(0)[..])
                .map_err(map_storage_error)?;
            metadata
                .insert(META_GLOBAL, &encode_counters(Counters::default())[..])
                .map_err(map_storage_error)?;
        }
        transaction.commit().map_err(map_commit_error)
    }

    fn validate_integrity(&self) -> Result<(), StorageError> {
        let transaction = self.database.begin_read().map_err(map_transaction_error)?;
        let objects = transaction.open_table(OBJECTS).map_err(map_table_error)?;
        let quotas = transaction
            .open_table(TENANT_QUOTAS)
            .map_err(map_table_error)?;
        let metadata = transaction.open_table(METADATA).map_err(map_table_error)?;
        let (revision, stored_global) = read_metadata(&metadata)?;

        let mut recomputed = BTreeMap::<TenantId, Counters>::new();
        let mut global = Counters::default();
        let mut maximum_version = 0;
        for row in objects.iter().map_err(map_storage_error)? {
            let (key, value) = row.map_err(map_storage_error)?;
            let (scope, object_key) = decode_object_key(key.value())?;
            if encode_object_key(&scope, &object_key)? != key.value() {
                return Err(StorageError::CorruptStore);
            }
            let (version, object_value) = decode_object_value(value.value())?;
            maximum_version = maximum_version.max(version.revision());
            let bytes = u64::try_from(object_value.as_bytes().len())
                .map_err(|_| StorageError::CorruptStore)?;
            global.objects = global
                .objects
                .checked_add(1)
                .ok_or(StorageError::CorruptStore)?;
            global.value_bytes = global
                .value_bytes
                .checked_add(bytes)
                .ok_or(StorageError::CorruptStore)?;
            if global.objects > self.limits.max_objects_global()
                || global.value_bytes > self.limits.max_value_bytes_global()
            {
                return Err(StorageError::CorruptStore);
            }
            let tenant = recomputed.entry(scope.tenant_id).or_default();
            tenant.objects = tenant
                .objects
                .checked_add(1)
                .ok_or(StorageError::CorruptStore)?;
            tenant.value_bytes = tenant
                .value_bytes
                .checked_add(bytes)
                .ok_or(StorageError::CorruptStore)?;
            if tenant.objects > self.limits.max_objects_per_tenant()
                || tenant.value_bytes > self.limits.max_value_bytes_per_tenant()
            {
                return Err(StorageError::CorruptStore);
            }
        }

        let maximum_quota_rows = global
            .objects
            .checked_add(1)
            .ok_or(StorageError::CorruptStore)?;
        let mut quota_rows = 0_u64;
        for row in quotas.iter().map_err(map_storage_error)? {
            quota_rows = quota_rows
                .checked_add(1)
                .ok_or(StorageError::CorruptStore)?;
            if quota_rows > maximum_quota_rows {
                return Err(StorageError::CorruptStore);
            }
            let (key, value) = row.map_err(map_storage_error)?;
            let tenant = decode_tenant_key(key.value())?;
            if encode_tenant_key(&tenant)? != key.value() {
                return Err(StorageError::CorruptStore);
            }
            let held = decode_counters(value.value(), true)?;
            if held.objects > self.limits.max_objects_per_tenant()
                || held.value_bytes > self.limits.max_value_bytes_per_tenant()
                || recomputed.remove(&tenant) != Some(held)
            {
                return Err(StorageError::CorruptStore);
            }
        }
        if !recomputed.is_empty()
            || global != stored_global
            || (global.objects > 0 && revision == 0)
            || revision < maximum_version
        {
            return Err(StorageError::CorruptStore);
        }
        Ok(())
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
            .ok_or(StorageError::CorruptStore)?
            .checked_add(new_bytes)
            .ok_or(StorageError::LimitExceeded)?;
        let global_bytes = global
            .value_bytes
            .checked_sub(old_bytes)
            .ok_or(StorageError::CorruptStore)?
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

impl ObjectStore for RedbObjectStore {
    fn get(
        &self,
        scope: &StorageScope,
        key: &ObjectKey,
    ) -> Result<Option<StoredObject>, StorageError> {
        let encoded = encode_object_key(scope, key)?;
        let transaction = self.database.begin_read().map_err(map_transaction_error)?;
        let objects = transaction.open_table(OBJECTS).map_err(map_table_error)?;
        objects
            .get(encoded.as_slice())
            .map_err(map_storage_error)?
            .map(|value| {
                let (version, value) = decode_object_value(value.value())?;
                Ok(StoredObject { version, value })
            })
            .transpose()
    }

    fn put(
        &self,
        scope: &StorageScope,
        key: ObjectKey,
        value: ObjectValue,
        condition: PutCondition,
    ) -> Result<PutOutcome, StorageError> {
        let object_key = encode_object_key(scope, &key)?;
        let tenant_key = encode_tenant_key(&scope.tenant_id)?;
        let new_bytes =
            u64::try_from(value.as_bytes().len()).map_err(|_| StorageError::OperationFailed)?;
        let mut transaction = self.database.begin_write().map_err(map_transaction_error)?;
        transaction.set_durability(Durability::Immediate);
        let outcome;
        {
            let mut objects = transaction.open_table(OBJECTS).map_err(map_table_error)?;
            let mut quotas = transaction
                .open_table(TENANT_QUOTAS)
                .map_err(map_table_error)?;
            let mut metadata = transaction.open_table(METADATA).map_err(map_table_error)?;
            let current = objects
                .get(object_key.as_slice())
                .map_err(map_storage_error)?
                .map(|held| held.value().to_vec());
            let current_decoded = current.as_deref().map(decode_object_value).transpose()?;
            let matches = match (&condition, current_decoded.as_ref()) {
                (PutCondition::Any, _) | (PutCondition::IfAbsent, None) => true,
                (PutCondition::IfVersion(expected), Some((version, _))) => expected == version,
                (PutCondition::IfAbsent | PutCondition::IfVersion(_), _) => false,
            };
            if !matches {
                return Ok(PutOutcome::Conflict);
            }
            let (revision, global) = read_metadata(&metadata)?;
            let revision = revision
                .checked_add(1)
                .ok_or(StorageError::RevisionExhausted)?;
            let version = ObjectVersion::from_revision(revision)?;
            let tenant = quotas
                .get(tenant_key.as_slice())
                .map_err(map_storage_error)?
                .map_or(Ok(Counters::default()), |held| {
                    decode_counters(held.value(), true)
                })?;
            let old_bytes = current_decoded
                .as_ref()
                .map(|(_, held)| {
                    u64::try_from(held.as_bytes().len()).map_err(|_| StorageError::CorruptStore)
                })
                .transpose()?;
            let (tenant, global) =
                self.checked_put_counters(tenant, global, old_bytes, new_bytes)?;
            let encoded_value = encode_object_value(&version, &value)?;
            objects
                .insert(object_key.as_slice(), encoded_value.as_slice())
                .map_err(map_storage_error)?;
            quotas
                .insert(tenant_key.as_slice(), &encode_counters(tenant)[..])
                .map_err(map_storage_error)?;
            metadata
                .insert(META_REVISION, &encode_revision(revision)[..])
                .map_err(map_storage_error)?;
            metadata
                .insert(META_GLOBAL, &encode_counters(global)[..])
                .map_err(map_storage_error)?;
            outcome = if current.is_none() {
                PutOutcome::Created { version }
            } else {
                PutOutcome::Replaced { version }
            };
        }
        transaction.commit().map_err(map_commit_error)?;
        Ok(outcome)
    }

    fn delete(
        &self,
        scope: &StorageScope,
        key: &ObjectKey,
        condition: DeleteCondition,
    ) -> Result<DeleteOutcome, StorageError> {
        let object_key = encode_object_key(scope, key)?;
        let tenant_key = encode_tenant_key(&scope.tenant_id)?;
        let mut transaction = self.database.begin_write().map_err(map_transaction_error)?;
        transaction.set_durability(Durability::Immediate);
        {
            let mut objects = transaction.open_table(OBJECTS).map_err(map_table_error)?;
            let mut quotas = transaction
                .open_table(TENANT_QUOTAS)
                .map_err(map_table_error)?;
            let mut metadata = transaction.open_table(METADATA).map_err(map_table_error)?;
            let Some(encoded_current) = objects
                .get(object_key.as_slice())
                .map_err(map_storage_error)?
                .map(|held| held.value().to_vec())
            else {
                return Ok(DeleteOutcome::NotFound);
            };
            let (version, value) = decode_object_value(&encoded_current)?;
            if let DeleteCondition::IfVersion(expected) = &condition
                && expected != &version
            {
                return Ok(DeleteOutcome::Conflict);
            }
            let (_, global) = read_metadata(&metadata)?;
            let tenant_value = quotas
                .get(tenant_key.as_slice())
                .map_err(map_storage_error)?
                .ok_or(StorageError::CorruptStore)?;
            let tenant = decode_counters(tenant_value.value(), true)?;
            drop(tenant_value);
            let bytes =
                u64::try_from(value.as_bytes().len()).map_err(|_| StorageError::CorruptStore)?;
            let tenant = Counters {
                objects: tenant
                    .objects
                    .checked_sub(1)
                    .ok_or(StorageError::CorruptStore)?,
                value_bytes: tenant
                    .value_bytes
                    .checked_sub(bytes)
                    .ok_or(StorageError::CorruptStore)?,
            };
            let global = Counters {
                objects: global
                    .objects
                    .checked_sub(1)
                    .ok_or(StorageError::CorruptStore)?,
                value_bytes: global
                    .value_bytes
                    .checked_sub(bytes)
                    .ok_or(StorageError::CorruptStore)?,
            };
            if tenant.objects == 0 && tenant.value_bytes != 0 {
                return Err(StorageError::CorruptStore);
            }
            objects
                .remove(object_key.as_slice())
                .map_err(map_storage_error)?;
            if tenant.objects == 0 {
                quotas
                    .remove(tenant_key.as_slice())
                    .map_err(map_storage_error)?;
            } else {
                quotas
                    .insert(tenant_key.as_slice(), &encode_counters(tenant)[..])
                    .map_err(map_storage_error)?;
            }
            metadata
                .insert(META_GLOBAL, &encode_counters(global)[..])
                .map_err(map_storage_error)?;
        }
        transaction.commit().map_err(map_commit_error)?;
        Ok(DeleteOutcome::Deleted)
    }

    fn list(&self, scope: &StorageScope, request: &ListRequest) -> Result<ListPage, StorageError> {
        let prefix = encode_scope_prefix(scope)?;
        let upper = prefix_successor(prefix.clone()).ok_or(StorageError::OperationFailed)?;
        let lower = request
            .after_key
            .as_ref()
            .map_or(Ok(prefix), |key| encode_object_key(scope, key))?;
        let transaction = self.database.begin_read().map_err(map_transaction_error)?;
        let objects_table = transaction.open_table(OBJECTS).map_err(map_table_error)?;
        let rows = objects_table
            .range::<&[u8]>((Excluded(lower.as_slice()), Excluded(upper.as_slice())))
            .map_err(map_storage_error)?;
        let limit = request.limit.as_usize()?;
        let mut objects = Vec::with_capacity(limit);
        let mut has_more = false;
        for row in rows {
            let (encoded_key, encoded_value) = row.map_err(map_storage_error)?;
            if objects.len() == limit {
                has_more = true;
                break;
            }
            let (decoded_scope, key) = decode_object_key(encoded_key.value())?;
            if &decoded_scope != scope {
                return Err(StorageError::CorruptStore);
            }
            let (version, value) = decode_object_value(encoded_value.value())?;
            objects.push(ObjectMetadata {
                key,
                version,
                size_bytes: u32::try_from(value.as_bytes().len())
                    .map_err(|_| StorageError::CorruptStore)?,
            });
        }
        Ok(ListPage { objects, has_more })
    }

    fn guarantees(&self) -> StoreGuarantees {
        StoreGuarantees {
            persistence: PersistenceGuarantee::ImmediateCommit,
            shared_across_processes: false,
            per_operation_atomic: true,
            conditional_writes: true,
            eviction: false,
            limits: self.limits,
        }
    }
}

#[derive(Clone, Copy)]
enum SchemaState {
    Absent,
    Present,
}

fn encode_scope_prefix(scope: &StorageScope) -> Result<Vec<u8>, StorageError> {
    let tenant = scope.tenant_id.as_str().as_bytes();
    let namespace = scope.namespace.as_str().as_bytes();
    let tenant_len = u16::try_from(tenant.len()).map_err(|_| StorageError::OperationFailed)?;
    let namespace_len =
        u16::try_from(namespace.len()).map_err(|_| StorageError::OperationFailed)?;
    let capacity = 1_usize
        .checked_add(2)
        .and_then(|value| value.checked_add(tenant.len()))
        .and_then(|value| value.checked_add(2))
        .and_then(|value| value.checked_add(namespace.len()))
        .ok_or(StorageError::OperationFailed)?;
    let mut encoded = Vec::with_capacity(capacity);
    encoded.push(SCHEMA);
    encoded.extend_from_slice(&tenant_len.to_be_bytes());
    encoded.extend_from_slice(tenant);
    encoded.extend_from_slice(&namespace_len.to_be_bytes());
    encoded.extend_from_slice(namespace);
    Ok(encoded)
}

fn encode_object_key(scope: &StorageScope, key: &ObjectKey) -> Result<Vec<u8>, StorageError> {
    let mut encoded = encode_scope_prefix(scope)?;
    encoded
        .len()
        .checked_add(key.as_bytes().len())
        .ok_or(StorageError::OperationFailed)?;
    encoded.extend_from_slice(key.as_bytes());
    Ok(encoded)
}

fn decode_object_key(encoded: &[u8]) -> Result<(StorageScope, ObjectKey), StorageError> {
    if encoded.first() != Some(&SCHEMA) {
        return Err(StorageError::CorruptStore);
    }
    let mut offset = 1;
    let tenant_length = read_u16(encoded, &mut offset)?;
    if tenant_length == 0 || tenant_length > MAX_TENANT_ID_BYTES {
        return Err(StorageError::CorruptStore);
    }
    let tenant = read_component(encoded, &mut offset, usize::from(tenant_length))?;
    let namespace_length = read_u16(encoded, &mut offset)?;
    if namespace_length == 0 || namespace_length > MAX_NAMESPACE_BYTES {
        return Err(StorageError::CorruptStore);
    }
    let namespace = read_component(encoded, &mut offset, usize::from(namespace_length))?;
    let key = encoded.get(offset..).ok_or(StorageError::CorruptStore)?;
    if key.is_empty() || key.len() > usize::from(MAX_OBJECT_KEY_BYTES) {
        return Err(StorageError::CorruptStore);
    }
    let tenant = std::str::from_utf8(tenant).map_err(|_| StorageError::CorruptStore)?;
    let namespace = std::str::from_utf8(namespace).map_err(|_| StorageError::CorruptStore)?;
    Ok((
        StorageScope::new(
            TenantId::new(tenant).map_err(|_| StorageError::CorruptStore)?,
            Namespace::new(namespace).map_err(|_| StorageError::CorruptStore)?,
        ),
        ObjectKey::new(key.to_vec()).map_err(|_| StorageError::CorruptStore)?,
    ))
}

fn encode_object_value(
    version: &ObjectVersion,
    value: &ObjectValue,
) -> Result<Vec<u8>, StorageError> {
    let capacity = 9_usize
        .checked_add(value.as_bytes().len())
        .ok_or(StorageError::OperationFailed)?;
    let mut encoded = Vec::with_capacity(capacity);
    encoded.push(SCHEMA);
    encoded.extend_from_slice(&version.revision().to_be_bytes());
    encoded.extend_from_slice(value.as_bytes());
    Ok(encoded)
}

fn decode_object_value(encoded: &[u8]) -> Result<(ObjectVersion, ObjectValue), StorageError> {
    if encoded.len() < 9
        || encoded[0] != SCHEMA
        || encoded.len() - 9 > usize::try_from(MAX_OBJECT_VALUE_BYTES).unwrap_or(usize::MAX)
    {
        return Err(StorageError::CorruptStore);
    }
    let revision = u64::from_be_bytes(
        encoded[1..9]
            .try_into()
            .map_err(|_| StorageError::CorruptStore)?,
    );
    Ok((
        ObjectVersion::from_revision(revision)?,
        ObjectValue::new(encoded[9..].to_vec()).map_err(|_| StorageError::CorruptStore)?,
    ))
}

fn encode_tenant_key(tenant: &TenantId) -> Result<Vec<u8>, StorageError> {
    let bytes = tenant.as_str().as_bytes();
    let length = u16::try_from(bytes.len()).map_err(|_| StorageError::OperationFailed)?;
    let mut encoded = Vec::with_capacity(
        3_usize
            .checked_add(bytes.len())
            .ok_or(StorageError::OperationFailed)?,
    );
    encoded.push(SCHEMA);
    encoded.extend_from_slice(&length.to_be_bytes());
    encoded.extend_from_slice(bytes);
    Ok(encoded)
}

fn decode_tenant_key(encoded: &[u8]) -> Result<TenantId, StorageError> {
    if encoded.first() != Some(&SCHEMA) {
        return Err(StorageError::CorruptStore);
    }
    let mut offset = 1;
    let length = read_u16(encoded, &mut offset)?;
    if length == 0 || length > MAX_TENANT_ID_BYTES || encoded.len() != 3 + usize::from(length) {
        return Err(StorageError::CorruptStore);
    }
    let value = std::str::from_utf8(&encoded[offset..]).map_err(|_| StorageError::CorruptStore)?;
    TenantId::new(value).map_err(|_| StorageError::CorruptStore)
}

fn encode_revision(revision: u64) -> [u8; 9] {
    let mut encoded = [0_u8; 9];
    encoded[0] = SCHEMA;
    encoded[1..].copy_from_slice(&revision.to_be_bytes());
    encoded
}

fn decode_revision(encoded: &[u8]) -> Result<u64, StorageError> {
    if encoded.len() != 9 || encoded[0] != SCHEMA {
        return Err(StorageError::CorruptStore);
    }
    Ok(u64::from_be_bytes(
        encoded[1..]
            .try_into()
            .map_err(|_| StorageError::CorruptStore)?,
    ))
}

fn encode_counters(counters: Counters) -> [u8; 17] {
    let mut encoded = [0_u8; 17];
    encoded[0] = SCHEMA;
    encoded[1..9].copy_from_slice(&counters.objects.to_be_bytes());
    encoded[9..17].copy_from_slice(&counters.value_bytes.to_be_bytes());
    encoded
}

fn decode_counters(encoded: &[u8], require_nonzero: bool) -> Result<Counters, StorageError> {
    if encoded.len() != 17 || encoded[0] != SCHEMA {
        return Err(StorageError::CorruptStore);
    }
    let counters = Counters {
        objects: u64::from_be_bytes(
            encoded[1..9]
                .try_into()
                .map_err(|_| StorageError::CorruptStore)?,
        ),
        value_bytes: u64::from_be_bytes(
            encoded[9..17]
                .try_into()
                .map_err(|_| StorageError::CorruptStore)?,
        ),
    };
    if require_nonzero && counters.objects == 0 {
        return Err(StorageError::CorruptStore);
    }
    Ok(counters)
}

fn read_metadata<T>(metadata: &T) -> Result<(u64, Counters), StorageError>
where
    T: ReadableTable<&'static [u8], &'static [u8]>,
{
    if metadata.len().map_err(map_storage_error)? != 3 {
        return Err(StorageError::CorruptStore);
    }
    let schema = metadata
        .get(META_SCHEMA)
        .map_err(map_storage_error)?
        .ok_or(StorageError::CorruptStore)?;
    if schema.value() != [SCHEMA] {
        return Err(StorageError::CorruptStore);
    }
    let revision = metadata
        .get(META_REVISION)
        .map_err(map_storage_error)?
        .ok_or(StorageError::CorruptStore)?;
    let revision = decode_revision(revision.value())?;
    let global = metadata
        .get(META_GLOBAL)
        .map_err(map_storage_error)?
        .ok_or(StorageError::CorruptStore)?;
    Ok((revision, decode_counters(global.value(), false)?))
}

fn prefix_successor(mut prefix: Vec<u8>) -> Option<Vec<u8>> {
    for byte in prefix.iter_mut().rev() {
        if *byte != u8::MAX {
            *byte += 1;
            return Some(prefix);
        }
        *byte = 0;
    }
    None
}

fn read_u16(encoded: &[u8], offset: &mut usize) -> Result<u16, StorageError> {
    let end = offset.checked_add(2).ok_or(StorageError::CorruptStore)?;
    let bytes = encoded
        .get(*offset..end)
        .ok_or(StorageError::CorruptStore)?;
    *offset = end;
    Ok(u16::from_be_bytes(
        bytes.try_into().map_err(|_| StorageError::CorruptStore)?,
    ))
}

fn read_component<'a>(
    encoded: &'a [u8],
    offset: &mut usize,
    length: usize,
) -> Result<&'a [u8], StorageError> {
    let end = offset
        .checked_add(length)
        .ok_or(StorageError::CorruptStore)?;
    let value = encoded
        .get(*offset..end)
        .ok_or(StorageError::CorruptStore)?;
    *offset = end;
    Ok(value)
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "map_err supplies owned redb errors and this mapper must not expose their details"
)]
fn map_database_error(error: DatabaseError) -> StorageError {
    match error {
        DatabaseError::DatabaseAlreadyOpen => StorageError::LockUnavailable,
        DatabaseError::Storage(redb::StorageError::Corrupted(_)) => StorageError::CorruptStore,
        _ => StorageError::OperationFailed,
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "map_err supplies owned redb errors and this mapper must not expose their details"
)]
fn map_transaction_error(error: redb::TransactionError) -> StorageError {
    match error {
        redb::TransactionError::Storage(error) => map_storage_error(error),
        _ => StorageError::OperationFailed,
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "map_err supplies owned redb errors and this mapper must not expose their details"
)]
fn map_commit_error(error: redb::CommitError) -> StorageError {
    match error {
        redb::CommitError::Storage(error) => map_storage_error(error),
        _ => StorageError::OperationFailed,
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "map_err supplies owned redb errors and this mapper must not expose their details"
)]
fn map_table_error(error: TableError) -> StorageError {
    match error {
        TableError::Storage(error) => map_storage_error(error),
        _ => StorageError::CorruptStore,
    }
}

#[allow(
    clippy::needless_pass_by_value,
    reason = "map_err supplies owned redb errors and this mapper must not expose their details"
)]
fn map_storage_error(error: redb::StorageError) -> StorageError {
    match error {
        redb::StorageError::Corrupted(_) => StorageError::CorruptStore,
        _ => StorageError::OperationFailed,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempPath(PathBuf);

    impl TempPath {
        fn new(label: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos();
            Self(std::env::temp_dir().join(format!(
                "storage-redb-{label}-{}-{nanos}-{nonce}.redb",
                std::process::id()
            )))
        }
    }

    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn limits() -> StorageLimits {
        StorageLimits::new(10, 100, 20, 200).expect("valid limits")
    }

    fn scope(tenant: &str, namespace: &str) -> StorageScope {
        StorageScope::new(
            TenantId::new(tenant).expect("valid tenant"),
            Namespace::new(namespace).expect("valid namespace"),
        )
    }

    fn initialize(path: &Path) {
        drop(RedbObjectStore::open(path, limits()).expect("initialize store"));
    }

    fn raw_write(path: &Path, mutation: impl FnOnce(&redb::WriteTransaction)) {
        let database = Database::create(path).expect("open raw database");
        let mut transaction = database.begin_write().expect("begin raw write");
        transaction.set_durability(Durability::Immediate);
        mutation(&transaction);
        transaction.commit().expect("commit raw write");
    }

    fn assert_corrupt(error: StorageError) {
        assert_eq!(error, StorageError::CorruptStore);
        assert_eq!(error.to_string(), "corrupt_store");
        assert_eq!(format!("{error:?}"), "corrupt_store");
    }

    #[test]
    fn golden_private_encodings_are_exact_and_key_order_is_preserved() {
        let scope = scope("A1", "n-x");
        let key = ObjectKey::new(vec![0, 0xff, 1]).unwrap();
        assert_eq!(
            encode_object_key(&scope, &key).unwrap(),
            vec![1, 0, 2, b'A', b'1', 0, 3, b'n', b'-', b'x', 0, 0xff, 1]
        );
        assert_eq!(
            encode_tenant_key(&scope.tenant_id).unwrap(),
            vec![1, 0, 2, b'A', b'1']
        );
        assert_eq!(encode_revision(0x0102), [1, 0, 0, 0, 0, 0, 0, 1, 2]);
        assert_eq!(
            encode_counters(Counters {
                objects: 1,
                value_bytes: 2,
            }),
            [1, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 2]
        );
        let version = ObjectVersion::from_revision(0x0102).unwrap();
        assert_eq!(
            encode_object_value(&version, &ObjectValue::new(vec![0xaa]).unwrap()).unwrap(),
            vec![1, 0, 0, 0, 0, 0, 0, 1, 2, 0xaa]
        );

        let mut raw = vec![vec![0xff], vec![0], vec![0, 0], vec![1]];
        let mut encoded = raw
            .iter()
            .map(|key| encode_object_key(&scope, &ObjectKey::new(key.clone()).unwrap()).unwrap())
            .collect::<Vec<_>>();
        raw.sort();
        encoded.sort();
        let decoded = encoded
            .iter()
            .map(|bytes| decode_object_key(bytes).unwrap().1.into_bytes_for_test())
            .collect::<Vec<_>>();
        assert_eq!(decoded, raw);
    }

    #[test]
    fn raw_database_contains_exact_golden_rows() {
        let path = TempPath::new("golden-rows");
        let store = RedbObjectStore::open(&path.0, limits()).unwrap();
        let scope = scope("t", "n");
        let version = match store
            .put(
                &scope,
                ObjectKey::new(b"k".to_vec()).unwrap(),
                ObjectValue::new(b"xy".to_vec()).unwrap(),
                PutCondition::Any,
            )
            .unwrap()
        {
            PutOutcome::Created { version } => version,
            outcome => panic!("unexpected outcome {outcome:?}"),
        };
        drop(store);

        let database = Database::create(&path.0).unwrap();
        let transaction = database.begin_read().unwrap();
        let objects = transaction.open_table(OBJECTS).unwrap();
        let object_key =
            encode_object_key(&scope, &ObjectKey::new(b"k".to_vec()).unwrap()).unwrap();
        assert_eq!(
            objects.get(object_key.as_slice()).unwrap().unwrap().value(),
            encode_object_value(&version, &ObjectValue::new(b"xy".to_vec()).unwrap()).unwrap()
        );
        let quotas = transaction.open_table(TENANT_QUOTAS).unwrap();
        let tenant_key = encode_tenant_key(&scope.tenant_id).unwrap();
        assert_eq!(
            quotas.get(tenant_key.as_slice()).unwrap().unwrap().value(),
            encode_counters(Counters {
                objects: 1,
                value_bytes: 2
            })
        );
        let metadata = transaction.open_table(METADATA).unwrap();
        assert_eq!(metadata.get(META_SCHEMA).unwrap().unwrap().value(), [1]);
        assert_eq!(
            metadata.get(META_REVISION).unwrap().unwrap().value(),
            encode_revision(1)
        );
        assert_eq!(
            metadata.get(META_GLOBAL).unwrap().unwrap().value(),
            encode_counters(Counters {
                objects: 1,
                value_bytes: 2
            })
        );
    }

    #[test]
    fn preexisting_empty_redb_is_rejected_without_storage_initialization_or_repair() {
        let path = TempPath::new("preexisting-empty");
        drop(Database::create(&path.0).expect("create empty raw database"));

        let error = RedbObjectStore::open(&path.0, limits())
            .err()
            .expect("preexisting empty database must not be treated as fresh");
        assert_corrupt(error);

        let database = Database::open(&path.0).expect("reopen unchanged raw database");
        let transaction = database.begin_read().expect("begin raw read");
        let table_names = transaction
            .list_tables()
            .expect("list raw tables")
            .map(|table| table.name().to_owned())
            .collect::<Vec<_>>();
        for storage_table in [OBJECTS.name(), TENANT_QUOTAS.name(), METADATA.name()] {
            assert!(
                !table_names.iter().any(|name| name == storage_table),
                "failed Storage open must not create {storage_table}"
            );
        }
        assert!(
            table_names.is_empty(),
            "raw database must remain free of application tables"
        );
    }

    #[test]
    fn genuinely_nonexistent_path_initializes_all_storage_tables() {
        let path = TempPath::new("nonexistent-initializes");
        assert!(!path.0.exists(), "test path must start nonexistent");

        drop(RedbObjectStore::open(&path.0, limits()).expect("initialize new store"));

        let database = Database::open(&path.0).expect("reopen initialized raw database");
        let transaction = database.begin_read().expect("begin raw read");
        let table_names = transaction
            .list_tables()
            .expect("list initialized tables")
            .map(|table| table.name().to_owned())
            .collect::<Vec<_>>();
        for storage_table in [OBJECTS.name(), TENANT_QUOTAS.name(), METADATA.name()] {
            assert!(
                table_names.iter().any(|name| name == storage_table),
                "fresh Storage open must create {storage_table}"
            );
        }
        assert_eq!(table_names.len(), 3, "only Storage tables are initialized");
    }

    #[test]
    fn clean_reopen_retains_values_versions_pages_counters_and_guarantees() {
        let path = TempPath::new("reopen");
        let scope = scope("t", "n");
        let first_version;
        {
            let store = RedbObjectStore::open(&path.0, limits()).unwrap();
            first_version = match store
                .put(
                    &scope,
                    ObjectKey::new(b"a".to_vec()).unwrap(),
                    ObjectValue::new(b"one".to_vec()).unwrap(),
                    PutCondition::Any,
                )
                .unwrap()
            {
                PutOutcome::Created { version } => version,
                outcome => panic!("unexpected {outcome:?}"),
            };
            store
                .put(
                    &scope,
                    ObjectKey::new(b"b".to_vec()).unwrap(),
                    ObjectValue::new(b"two".to_vec()).unwrap(),
                    PutCondition::Any,
                )
                .unwrap();
        }
        let reopened = RedbObjectStore::open(&path.0, limits()).unwrap();
        let held = reopened
            .get(&scope, &ObjectKey::new(b"a".to_vec()).unwrap())
            .unwrap()
            .unwrap();
        assert_eq!(held.version, first_version);
        assert_eq!(held.value.as_bytes(), b"one");
        let page = reopened
            .list(
                &scope,
                &ListRequest::new(None, crate::ListLimit::new(1).unwrap()),
            )
            .unwrap();
        assert_eq!(page.objects.len(), 1);
        assert_eq!(page.objects[0].key.as_bytes(), b"a");
        assert!(page.has_more);
        assert_eq!(
            reopened.guarantees().persistence,
            PersistenceGuarantee::ImmediateCommit
        );
    }

    #[test]
    fn exclusive_lock_contention_is_redacted_and_released_on_drop() {
        let path = TempPath::new("lock");
        let store = RedbObjectStore::open(&path.0, limits()).unwrap();
        let error = RedbObjectStore::open(&path.0, limits())
            .err()
            .expect("second open must fail");
        assert_eq!(error, StorageError::LockUnavailable);
        assert_eq!(error.to_string(), "lock_unavailable");
        assert_eq!(format!("{error:?}"), "lock_unavailable");
        assert!(
            !error
                .to_string()
                .contains(path.0.to_string_lossy().as_ref())
        );
        drop(store);
        assert!(RedbObjectStore::open(&path.0, limits()).is_ok());
    }

    #[test]
    fn partial_and_extra_tables_are_rejected() {
        const EXTRA: TableDefinition<'static, &'static [u8], &'static [u8]> =
            TableDefinition::new("unexpected");
        let partial = TempPath::new("partial");
        raw_write(&partial.0, |transaction| {
            transaction.open_table(OBJECTS).unwrap();
        });
        assert_corrupt(
            RedbObjectStore::open(&partial.0, limits())
                .err()
                .expect("partial schema rejected"),
        );

        let extra = TempPath::new("extra");
        initialize(&extra.0);
        raw_write(&extra.0, |transaction| {
            transaction.open_table(EXTRA).unwrap();
        });
        assert_corrupt(
            RedbObjectStore::open(&extra.0, limits())
                .err()
                .expect("extra table rejected"),
        );
    }

    #[test]
    fn catalog_with_more_than_three_extra_tables_is_rejected_and_redacted() {
        const EXTRA_A: TableDefinition<'static, &'static [u8], &'static [u8]> =
            TableDefinition::new("extra_a");
        const EXTRA_B: TableDefinition<'static, &'static [u8], &'static [u8]> =
            TableDefinition::new("extra_b");
        const EXTRA_C: TableDefinition<'static, &'static [u8], &'static [u8]> =
            TableDefinition::new("extra_c");
        const EXTRA_D: TableDefinition<'static, &'static [u8], &'static [u8]> =
            TableDefinition::new("extra_d");

        let path = TempPath::new("many-extra-tables");
        initialize(&path.0);
        raw_write(&path.0, |transaction| {
            transaction.open_table(EXTRA_A).unwrap();
            transaction.open_table(EXTRA_B).unwrap();
            transaction.open_table(EXTRA_C).unwrap();
            transaction.open_table(EXTRA_D).unwrap();
        });

        let error = RedbObjectStore::open(&path.0, limits())
            .err()
            .expect("catalog exceeding the three-table schema must be rejected");
        assert_corrupt(error);
        let display = error.to_string();
        assert!(!display.contains(path.0.to_string_lossy().as_ref()));
        for table_name in ["extra_a", "extra_b", "extra_c", "extra_d"] {
            assert!(!display.contains(table_name));
        }
    }

    #[test]
    fn malformed_metadata_and_extra_metadata_are_rejected_without_repair() {
        for (label, key, bytes) in [
            ("schema", META_SCHEMA, vec![2]),
            ("revision", META_REVISION, vec![1, 0]),
            ("global", META_GLOBAL, vec![1, 0]),
            ("extra-meta", &[0x04][..], vec![1]),
        ] {
            let path = TempPath::new(label);
            initialize(&path.0);
            raw_write(&path.0, |transaction| {
                let mut metadata = transaction.open_table(METADATA).unwrap();
                metadata.insert(key, bytes.as_slice()).unwrap();
            });
            assert_corrupt(
                RedbObjectStore::open(&path.0, limits())
                    .err()
                    .expect("malformed metadata rejected"),
            );
            let database = Database::create(&path.0).unwrap();
            let read = database.begin_read().unwrap();
            let metadata = read.open_table(METADATA).unwrap();
            assert_eq!(
                metadata.get(key).unwrap().unwrap().value(),
                bytes,
                "failed open repaired {label}"
            );
        }
    }

    #[test]
    fn malformed_object_key_value_zero_version_and_revision_regression_are_rejected() {
        for label in ["bad-key", "bad-value", "zero-version", "low-revision"] {
            let path = TempPath::new(label);
            let scope = scope("t", "n");
            {
                let store = RedbObjectStore::open(&path.0, limits()).unwrap();
                store
                    .put(
                        &scope,
                        ObjectKey::new(b"k".to_vec()).unwrap(),
                        ObjectValue::new(Vec::new()).unwrap(),
                        PutCondition::Any,
                    )
                    .unwrap();
            }
            raw_write(&path.0, |transaction| {
                if label == "low-revision" {
                    let mut metadata = transaction.open_table(METADATA).unwrap();
                    metadata
                        .insert(META_REVISION, &encode_revision(0)[..])
                        .unwrap();
                    return;
                }
                let mut objects = transaction.open_table(OBJECTS).unwrap();
                let valid_key =
                    encode_object_key(&scope, &ObjectKey::new(b"k".to_vec()).unwrap()).unwrap();
                if label == "bad-key" {
                    objects
                        .insert(
                            &[2, 0, 1, b't', 0, 1, b'n', b'k'][..],
                            &[1, 0, 0, 0, 0, 0, 0, 0, 1][..],
                        )
                        .unwrap();
                } else if label == "bad-value" {
                    objects
                        .insert(valid_key.as_slice(), &[2, 0, 0, 0, 0, 0, 0, 0, 1][..])
                        .unwrap();
                } else {
                    objects
                        .insert(valid_key.as_slice(), &[1, 0, 0, 0, 0, 0, 0, 0, 0][..])
                        .unwrap();
                }
            });
            assert_corrupt(
                RedbObjectStore::open(&path.0, limits())
                    .err()
                    .expect("malformed metadata rejected"),
            );
        }
    }

    #[test]
    fn quota_counter_mismatch_and_tightened_over_limit_state_are_rejected() {
        let mismatch = TempPath::new("counter-mismatch");
        let scope = scope("t", "n");
        {
            let store = RedbObjectStore::open(&mismatch.0, limits()).unwrap();
            store
                .put(
                    &scope,
                    ObjectKey::new(b"k".to_vec()).unwrap(),
                    ObjectValue::new(b"x".to_vec()).unwrap(),
                    PutCondition::Any,
                )
                .unwrap();
        }
        raw_write(&mismatch.0, |transaction| {
            let mut quotas = transaction.open_table(TENANT_QUOTAS).unwrap();
            let tenant_key = encode_tenant_key(&scope.tenant_id).unwrap();
            quotas
                .insert(
                    tenant_key.as_slice(),
                    &encode_counters(Counters {
                        objects: 1,
                        value_bytes: 2,
                    })[..],
                )
                .unwrap();
        });
        assert_corrupt(
            RedbObjectStore::open(&mismatch.0, limits())
                .err()
                .expect("counter mismatch rejected"),
        );

        let over = TempPath::new("over-limit");
        {
            let store = RedbObjectStore::open(&over.0, limits()).unwrap();
            store
                .put(
                    &scope,
                    ObjectKey::new(b"a".to_vec()).unwrap(),
                    ObjectValue::new(b"x".to_vec()).unwrap(),
                    PutCondition::Any,
                )
                .unwrap();
            store
                .put(
                    &scope,
                    ObjectKey::new(b"b".to_vec()).unwrap(),
                    ObjectValue::new(b"y".to_vec()).unwrap(),
                    PutCondition::Any,
                )
                .unwrap();
        }
        let tight = StorageLimits::new(1, 100, 20, 200).unwrap();
        assert_corrupt(
            RedbObjectStore::open(&over.0, tight)
                .err()
                .expect("over-limit state rejected"),
        );
        assert!(
            RedbObjectStore::open(&over.0, limits()).is_ok(),
            "failed tight open must not repair or erase state"
        );
    }

    #[test]
    fn persisted_revision_exhaustion_is_condition_first_and_has_no_trace() {
        let path = TempPath::new("revision-max");
        let scope = scope("t", "n");
        let key = ObjectKey::new(b"k".to_vec()).unwrap();
        {
            let store = RedbObjectStore::open(&path.0, limits()).unwrap();
            store
                .put(
                    &scope,
                    key.clone(),
                    ObjectValue::new(Vec::new()).unwrap(),
                    PutCondition::Any,
                )
                .unwrap();
        }
        raw_write(&path.0, |transaction| {
            let mut metadata = transaction.open_table(METADATA).unwrap();
            metadata
                .insert(META_REVISION, &encode_revision(u64::MAX)[..])
                .unwrap();
        });
        let store = RedbObjectStore::open(&path.0, limits()).unwrap();
        assert_eq!(
            store.put(
                &scope,
                key.clone(),
                ObjectValue::new(vec![1]).unwrap(),
                PutCondition::IfAbsent
            ),
            Ok(PutOutcome::Conflict)
        );
        assert_eq!(
            store.put(
                &scope,
                key.clone(),
                ObjectValue::new(vec![1]).unwrap(),
                PutCondition::Any
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
        drop(store);
        let reopened = RedbObjectStore::open(&path.0, limits()).unwrap();
        assert!(
            reopened
                .get(&scope, &key)
                .unwrap()
                .unwrap()
                .value
                .as_bytes()
                .is_empty()
        );
    }

    fn production_function_body<'a>(source: &'a str, function: &str) -> &'a str {
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("production implementation source");
        let signature = format!("fn {function}");
        let function_start = implementation
            .find(&signature)
            .unwrap_or_else(|| panic!("production function {function} exists"));
        let function_tail = &implementation[function_start..];
        let opening_brace = function_tail
            .find('{')
            .unwrap_or_else(|| panic!("production function {function} has a body"));
        let mut depth = 0_u32;
        for (offset, character) in function_tail[opening_brace..].char_indices() {
            match character {
                '{' => depth += 1,
                '}' => {
                    depth = depth.checked_sub(1).expect("balanced function braces");
                    if depth == 0 {
                        return &function_tail[opening_brace + 1..opening_brace + offset];
                    }
                }
                _ => {}
            }
        }
        panic!("production function {function} has balanced braces");
    }

    #[test]
    fn schema_state_keeps_catalog_scan_bounded_and_allocation_free() {
        let body = production_function_body(include_str!("redb.rs"), "schema_state");
        for forbidden in ["BTreeSet", ".collect", "to_owned"] {
            assert!(
                !body.contains(forbidden),
                "schema_state must not use {forbidden} for catalog names"
            );
        }

        let compact = body
            .chars()
            .filter(|character| !character.is_whitespace())
            .collect::<String>();
        let table_limit = compact
            .find("table_count>3")
            .expect("schema_state has a hard three-table catalog limit");
        let table_name = compact
            .find("table.name()")
            .expect("schema_state inspects allowed table names");
        assert!(
            table_limit < table_name,
            "table-count rejection must happen before catalog-name inspection"
        );
        assert!(
            compact[table_limit..table_name].contains("returnErr(StorageError::CorruptStore)"),
            "excess normal tables must be rejected immediately"
        );

        let multimap_scan = compact
            .find("list_multimap_tables()")
            .expect("schema_state inspects multimap tables");
        let multimap_tail = &compact[multimap_scan..];
        let first = multimap_tail
            .find(".next()")
            .expect("schema_state reads only the first multimap catalog entry");
        let present = multimap_tail[first..]
            .find(".is_some()")
            .expect("schema_state rejects a present first multimap entry");
        assert!(
            multimap_tail[first + present..].contains("returnErr(StorageError::CorruptStore)"),
            "the first multimap table must cause corruption rejection"
        );
    }

    #[test]
    fn every_adapter_mutation_selects_immediate_durability_before_commit() {
        let source = include_str!("redb.rs");
        let implementation = source
            .split("#[cfg(test)]")
            .next()
            .expect("implementation source");
        assert_eq!(
            implementation
                .matches("set_durability(Durability::Immediate)")
                .count(),
            3
        );
        for function in ["fn initialize", "fn put", "fn delete"] {
            let start = source.find(function).expect("mutation function exists");
            let tail = &source[start..];
            let durability = tail
                .find("set_durability(Durability::Immediate)")
                .expect("durability selection");
            let commit = tail.find("transaction.commit()").expect("commit");
            assert!(
                durability < commit,
                "{function} must select Immediate before commit"
            );
        }
    }

    trait ObjectKeyTestBytes {
        fn into_bytes_for_test(self) -> Vec<u8>;
    }

    impl ObjectKeyTestBytes for ObjectKey {
        fn into_bytes_for_test(self) -> Vec<u8> {
            self.as_bytes().to_vec()
        }
    }
}

#[cfg(test)]
mod additional_integrity_tests {
    use super::*;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TempPath(PathBuf);
    impl TempPath {
        fn new(label: &str) -> Self {
            static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
            let nonce = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let nanos = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_nanos();
            Self(std::env::temp_dir().join(format!(
                "storage-redb-extra-{label}-{}-{nanos}-{nonce}.redb",
                std::process::id()
            )))
        }
    }
    impl Drop for TempPath {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn limits() -> StorageLimits {
        StorageLimits::new(10, 2_000_000, 10, 2_000_000).unwrap()
    }
    fn scope() -> StorageScope {
        StorageScope::new(TenantId::new("t").unwrap(), Namespace::new("n").unwrap())
    }
    fn initialize(path: &Path) {
        drop(RedbObjectStore::open(path, limits()).unwrap());
    }
    fn raw_write(path: &Path, mutation: impl FnOnce(&redb::WriteTransaction)) {
        let database = Database::create(path).unwrap();
        let mut transaction = database.begin_write().unwrap();
        transaction.set_durability(Durability::Immediate);
        mutation(&transaction);
        transaction.commit().unwrap();
    }
    fn rejected(path: &Path) {
        assert_eq!(
            RedbObjectStore::open(path, limits())
                .err()
                .expect("corruption rejected"),
            StorageError::CorruptStore
        );
    }

    #[test]
    fn missing_metadata_record_is_rejected_without_recreation() {
        let path = TempPath::new("missing-meta");
        initialize(&path.0);
        raw_write(&path.0, |transaction| {
            transaction
                .open_table(METADATA)
                .unwrap()
                .remove(META_GLOBAL)
                .unwrap();
        });
        rejected(&path.0);
        let database = Database::create(&path.0).unwrap();
        let read = database.begin_read().unwrap();
        assert!(
            read.open_table(METADATA)
                .unwrap()
                .get(META_GLOBAL)
                .unwrap()
                .is_none()
        );
    }

    #[test]
    fn malformed_quota_key_and_value_are_rejected() {
        for label in ["quota-key", "quota-value"] {
            let path = TempPath::new(label);
            initialize(&path.0);
            raw_write(&path.0, |transaction| {
                let mut quotas = transaction.open_table(TENANT_QUOTAS).unwrap();
                if label == "quota-key" {
                    quotas
                        .insert(
                            &[2, 0, 1, b't'][..],
                            &encode_counters(Counters {
                                objects: 1,
                                value_bytes: 0,
                            })[..],
                        )
                        .unwrap();
                } else {
                    quotas.insert(&[1, 0, 1, b't'][..], &[2, 0][..]).unwrap();
                }
            });
            rejected(&path.0);
        }
    }

    #[test]
    fn oversized_persisted_object_value_is_rejected() {
        let path = TempPath::new("oversized-value");
        initialize(&path.0);
        let scope = scope();
        let key = encode_object_key(&scope, &ObjectKey::new(b"k".to_vec()).unwrap()).unwrap();
        let mut oversized = vec![0_u8; 9 + usize::try_from(MAX_OBJECT_VALUE_BYTES).unwrap() + 1];
        oversized[0] = SCHEMA;
        oversized[8] = 1;
        raw_write(&path.0, |transaction| {
            transaction
                .open_table(OBJECTS)
                .unwrap()
                .insert(key.as_slice(), oversized.as_slice())
                .unwrap();
        });
        rejected(&path.0);
    }

    #[test]
    fn runtime_counter_underflow_is_corruption_and_aborts_without_trace() {
        let path = TempPath::new("underflow");
        let store = RedbObjectStore::open(&path.0, limits()).unwrap();
        let scope = scope();
        let key = ObjectKey::new(b"k".to_vec()).unwrap();
        store
            .put(
                &scope,
                key.clone(),
                ObjectValue::new(b"x".to_vec()).unwrap(),
                PutCondition::Any,
            )
            .unwrap();
        {
            let mut transaction = store.database.begin_write().unwrap();
            transaction.set_durability(Durability::Immediate);
            let tenant_key = encode_tenant_key(&scope.tenant_id).unwrap();
            transaction
                .open_table(TENANT_QUOTAS)
                .unwrap()
                .insert(
                    tenant_key.as_slice(),
                    &encode_counters(Counters {
                        objects: 1,
                        value_bytes: 0,
                    })[..],
                )
                .unwrap();
            transaction.commit().unwrap();
        }
        assert_eq!(
            store.delete(&scope, &key, DeleteCondition::Any),
            Err(StorageError::CorruptStore)
        );
        assert_eq!(
            store.get(&scope, &key).unwrap().unwrap().value.as_bytes(),
            b"x"
        );
    }
}
