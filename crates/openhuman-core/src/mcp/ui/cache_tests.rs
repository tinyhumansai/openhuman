use super::*;
use crate::mcp::ui::types::UiCsp;

fn resource(html: &str) -> UiResource {
    UiResource {
        html: html.into(),
        mime_type: "text/html".into(),
        csp: UiCsp::default(),
        permissions: None,
        prefers_border: false,
    }
}

#[test]
fn inline_documents_round_trip_by_id() {
    let id = put_inline(InlineEntry {
        server_id: Some("cache-roundtrip".into()),
        resource: resource("<b/>"),
    });
    let entry = get_inline(&id).expect("stored");
    assert_eq!(entry.server_id.as_deref(), Some("cache-roundtrip"));
    assert_eq!(entry.resource.html, "<b/>");
    assert!(get_inline("absent").is_none());
}

#[test]
fn reads_are_keyed_by_server_and_uri() {
    put_read("cache-reads", "ui://a", resource("<a/>"));
    assert_eq!(get_read("cache-reads", "ui://a").unwrap().html, "<a/>");
    assert!(get_read("cache-reads", "ui://b").is_none());
    assert!(get_read("other", "ui://a").is_none());
}

#[test]
fn eviction_drops_the_oldest_entries_at_capacity() {
    let mut map: HashMap<u32, Timed<u32>> = HashMap::new();
    let base = Instant::now();
    for key in 0..MAX_ENTRIES as u32 {
        map.insert(
            key,
            Timed {
                at: base + Duration::from_millis(u64::from(key)),
                value: key,
            },
        );
    }
    evict(&mut map, Duration::from_secs(60));
    assert!(map.len() < MAX_ENTRIES);
    assert!(map.contains_key(&(MAX_ENTRIES as u32 - 1)));
    assert!(!map.contains_key(&0));
}

#[test]
fn eviction_drops_expired_entries() {
    let mut map: HashMap<u32, Timed<u32>> = HashMap::new();
    map.insert(
        1,
        Timed {
            at: Instant::now() - Duration::from_secs(10),
            value: 1,
        },
    );
    evict(&mut map, Duration::from_secs(1));
    assert!(map.is_empty());
}

#[test]
fn entries_are_not_shared_across_tenants() {
    use crate::core::runtime::Tenant;
    let tenant = |profile: Option<&str>| Tenant {
        profile: profile.map(str::to_string),
        agent: None,
    };
    assert_ne!(
        scoped_for(&tenant(Some("alice")), "doc"),
        scoped_for(&tenant(Some("bob")), "doc")
    );
    assert_ne!(
        scoped_for(&tenant(Some("alice")), "doc"),
        scoped_for(&tenant(None), "doc")
    );
    assert_eq!(scoped_for(&tenant(None), "doc"), "doc");
}
