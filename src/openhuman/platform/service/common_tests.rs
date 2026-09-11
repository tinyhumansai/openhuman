use super::*;

#[test]
fn xml_escape_replaces_entities() {
    let raw = "<tag>\"&'";
    let escaped = xml_escape(raw);
    assert!(escaped.contains("&lt;tag&gt;"));
    assert!(escaped.contains("&quot;"));
    assert!(escaped.contains("&amp;"));
    assert!(escaped.contains("&apos;"));
}
