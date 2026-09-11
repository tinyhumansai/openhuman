/// Redact sensitive values for safe logging. Shows first 4 chars + "***" suffix.
/// This function intentionally breaks the data-flow taint chain for static analysis.
pub fn redact(value: &str) -> String {
    if value.chars().count() <= 4 {
        "***".to_string()
    } else {
        crate::openhuman::util::truncate_with_suffix(value, 4, "***")
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
