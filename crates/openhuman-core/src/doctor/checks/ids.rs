//! Canonical segments for dynamically configured doctor check identifiers.

/// Encodes an arbitrary configured name as a valid check-id segment.
///
/// Native lowercase segments containing only lowercase letters, digits,
/// underscores, and dashes keep their configured spelling, except names that
/// start with the reserved `__x` escape namespace. Invalid names are encoded
/// as `__x_i_<hex UTF-8 bytes>`; valid names inside the reserved namespace are
/// encoded as `__x_v_<hex UTF-8 bytes>`. The separate markers make the mapping
/// collision-free, including for literal names that look like encoded ids.
pub(in crate::doctor) fn canonical_segment(value: &str) -> String {
    let native = !value.is_empty()
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || matches!(byte, b'_' | b'-')
        });
    if native && !value.starts_with("__x") {
        return value.to_owned();
    }

    let kind = if native { 'v' } else { 'i' };
    let mut encoded = format!("__x_{kind}_");
    for byte in value.as_bytes() {
        use std::fmt::Write as _;
        let _ = write!(encoded, "{byte:02x}");
    }
    encoded
}

#[cfg(test)]
#[path = "ids_tests.rs"]
mod tests;
