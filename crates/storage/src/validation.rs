use crate::error::StorageError;
use crate::model::{
    MAX_NAMESPACE_BYTES, MAX_OBJECT_KEY_BYTES, MAX_OBJECT_VALUE_BYTES, MAX_OBJECTS_GLOBAL,
    MAX_OBJECTS_PER_TENANT, MAX_TENANT_ID_BYTES, MAX_VALUE_BYTES_GLOBAL,
    MAX_VALUE_BYTES_PER_TENANT,
};

pub(crate) fn validate_logical_id(
    value: &str,
    maximum: u16,
    error: StorageError,
) -> Result<(), StorageError> {
    let bytes = value.as_bytes();
    let length = u16::try_from(bytes.len()).map_err(|_| error)?;
    if length == 0
        || length > maximum
        || !bytes[0].is_ascii_alphanumeric()
        || !bytes
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(error);
    }
    Ok(())
}

pub(crate) fn validate_tenant_id(value: &str) -> Result<(), StorageError> {
    validate_logical_id(value, MAX_TENANT_ID_BYTES, StorageError::InvalidTenantId)
}

pub(crate) fn validate_namespace(value: &str) -> Result<(), StorageError> {
    validate_logical_id(value, MAX_NAMESPACE_BYTES, StorageError::InvalidNamespace)
}

pub(crate) fn validate_key(value: &[u8]) -> Result<u16, StorageError> {
    let length = u16::try_from(value.len()).map_err(|_| StorageError::InvalidObjectKey)?;
    if length == 0 || length > MAX_OBJECT_KEY_BYTES {
        return Err(StorageError::InvalidObjectKey);
    }
    Ok(length)
}

pub(crate) fn validate_value(value: &[u8]) -> Result<u32, StorageError> {
    let length = u32::try_from(value.len()).map_err(|_| StorageError::InvalidValue)?;
    if length > MAX_OBJECT_VALUE_BYTES {
        return Err(StorageError::InvalidValue);
    }
    Ok(length)
}

pub(crate) fn validate_limits(
    max_objects_per_tenant: u64,
    max_value_bytes_per_tenant: u64,
    max_objects_global: u64,
    max_value_bytes_global: u64,
) -> Result<(), StorageError> {
    if max_objects_per_tenant == 0
        || max_objects_per_tenant > MAX_OBJECTS_PER_TENANT
        || max_value_bytes_per_tenant == 0
        || max_value_bytes_per_tenant > MAX_VALUE_BYTES_PER_TENANT
        || max_objects_global == 0
        || max_objects_global > MAX_OBJECTS_GLOBAL
        || max_value_bytes_global == 0
        || max_value_bytes_global > MAX_VALUE_BYTES_GLOBAL
        || max_objects_per_tenant > max_objects_global
        || max_value_bytes_per_tenant > max_value_bytes_global
    {
        return Err(StorageError::InvalidLimits);
    }
    Ok(())
}
