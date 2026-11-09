mod common;
#[path = "support/attached_tools.rs"]
mod scenarios;

#[test]
fn permanent_metadata_requires_a_real_source() {
    scenarios::permanent_metadata_requires_a_real_source();
}
