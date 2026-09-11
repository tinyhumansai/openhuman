#[test]
fn progress_timestamp_preserves_rfc3339_wire_format() {
    let timestamp = super::progress_updated_at();
    assert!(
        chrono::DateTime::parse_from_rfc3339(&timestamp).is_ok(),
        "progress-event updated_at must remain RFC 3339"
    );
}
