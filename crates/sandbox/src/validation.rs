use std::path::{Component, Path};

pub(crate) fn logical_id(value: &str) -> bool {
    let mut bytes = value.bytes();
    !value.is_empty()
        && value.len() <= crate::MAX_ID_BYTES
        && matches!(bytes.next(), Some(byte) if byte.is_ascii_lowercase() || byte.is_ascii_digit())
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'-' | b'_')
        })
}

pub(crate) fn sandbox_id(value: &str) -> bool {
    value.len() == 36
        && value.starts_with("sbx-")
        && value[4..]
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

pub(crate) fn bounded_text(value: &str, maximum: usize, allow_empty: bool) -> bool {
    (allow_empty || !value.is_empty()) && value.len() <= maximum && !value.contains('\0')
}

pub(crate) fn relative_path(value: &str) -> bool {
    bounded_text(value, crate::MAX_WORKING_DIRECTORY_BYTES, true)
        && (value.is_empty()
            || Path::new(value)
                .components()
                .all(|component| matches!(component, Component::Normal(_))))
}

pub(crate) fn immutable_image(value: &str) -> bool {
    let Some((name, digest)) = value.rsplit_once("@sha256:") else {
        return false;
    };
    !name.is_empty()
        && value.len() <= crate::MAX_IMAGE_BYTES
        && !name.contains(char::is_whitespace)
        && digest.len() == 64
        && digest
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}
