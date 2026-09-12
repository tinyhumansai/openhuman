//! Unit tests for the host-owned validating mutation surface.
//!
//! Ported from the engine's `memory/goals/mutations_tests.rs` — the same cases
//! against the same predicates, rewritten from method syntax (`doc.add(..)`) to
//! the free functions this module exposes. They are kept here rather than
//! trusted to the engine's copy because the predicates the guards call are now
//! reached from this crate, and a divergence would otherwise only surface as a
//! goal with an email address in it.

use super::*;

#[test]
fn parse_round_trips_render() {
    let mut doc = GoalsDoc::default();
    add_item(&mut doc, "ship the desktop app").unwrap();
    add_item(&mut doc, "keep the rust core authoritative").unwrap();
    let rendered = doc.render();
    let reparsed = GoalsDoc::parse(&rendered);
    assert_eq!(doc, reparsed);
}

#[test]
fn add_assigns_unique_ids() {
    let mut doc = GoalsDoc::default();
    let a = add_item(&mut doc, "a").unwrap();
    let b = add_item(&mut doc, "b").unwrap();
    assert_ne!(a, b);
    assert_eq!(doc.items.len(), 2);
}

#[test]
fn add_rejects_empty_text() {
    let mut doc = GoalsDoc::default();
    let err = add_item(&mut doc, "   ").unwrap_err();
    assert!(
        err.to_string().contains("goal text must not be empty"),
        "the empty-text message reaches the agent verbatim: {err}"
    );
}

#[test]
fn add_and_edit_reject_multiline_text() {
    let mut doc = GoalsDoc::default();
    // A newline-bearing goal would inject extra "- [..]" list lines on reload,
    // corrupting the stored shape — reject it outright.
    assert!(add_item(&mut doc, "line one\n- [x] injected").is_err());
    let id = add_item(&mut doc, "legit goal").unwrap();
    assert!(edit_item(&mut doc, &id, "still\rinjected").is_err());
}

#[test]
fn add_and_edit_reject_secret_or_pii_text() {
    let mut doc = GoalsDoc::default();
    assert!(add_item(&mut doc, "follow up with alice@example.com about launch").is_err());
    assert!(add_item(
        &mut doc,
        "rotate api_key=sk-abcdefghijklmnopqrstuvwxyz123456"
    )
    .is_err());

    let id = add_item(&mut doc, "ship the memory engine").unwrap();
    assert!(edit_item(&mut doc, &id, "call +14155551212 tomorrow").is_err());
    assert_eq!(
        doc.items[0].text, "ship the memory engine",
        "a rejected edit must leave the item untouched"
    );
}

#[test]
fn edit_updates_known_id_and_rejects_unknown() {
    let mut doc = GoalsDoc::default();
    let id = add_item(&mut doc, "old").unwrap();
    edit_item(&mut doc, &id, "new").unwrap();
    assert_eq!(doc.items[0].text, "new");
    assert!(edit_item(&mut doc, "nope", "x").is_err());
}

#[test]
fn delete_removes_known_id_and_rejects_unknown() {
    let mut doc = GoalsDoc::default();
    let id = add_item(&mut doc, "x").unwrap();
    delete_item(&mut doc, &id).unwrap();
    assert!(doc.is_empty());
    assert!(delete_item(&mut doc, "nope").is_err());
}

#[test]
fn delete_removes_only_the_first_occurrence_of_a_duplicate_id() {
    // A well-formed document never has duplicate ids, but `GoalsDoc::parse`
    // accepts a hand-edited or corrupt file that does. `delete` must not
    // bulk-remove every item sharing the id.
    let mut doc = GoalsDoc {
        items: vec![GoalItem::new("g1", "first"), GoalItem::new("g1", "second")],
    };
    delete_item(&mut doc, "g1").unwrap();
    assert_eq!(doc.items.len(), 1);
    assert_eq!(doc.items[0].text, "second");
}

#[test]
fn next_id_avoids_collision_with_custom_ids() {
    let mut doc = GoalsDoc {
        items: vec![GoalItem::new("g1", "a"), GoalItem::new("g2", "b")],
    };
    let id = add_item(&mut doc, "c").unwrap();
    assert_eq!(id, "g3");
}

/// The one property the whole module exists for: the guards the host now runs
/// are the same guards the engine ran, so a text the engine's `save` would
/// refuse never reaches it.
#[test]
fn host_guards_agree_with_the_engine_choke_point() {
    // The token is assembled at runtime so the source never contains a
    // contiguous credential-shaped string: secret scanners (tinysweeper's
    // github-personal-access-token rule among them) fire on the pattern in a
    // committed file, and they cannot tell a synthetic guard fixture from a
    // leak. The concatenated value is identical, so the guard under test
    // still sees a real token shape.
    let synthetic_token = format!(
        "store token {}{}",
        "ghp_", "abcdefghijklmnopqrstuvwxyz0123456789"
    );
    for rejected in ["email bob@example.org the plan", synthetic_token.as_str()] {
        let mut doc = GoalsDoc::default();
        assert!(
            add_item(&mut doc, rejected).is_err(),
            "host validation must reject {rejected:?} before `set_goals` sees it"
        );
        assert!(
            doc.is_empty(),
            "a rejected add must not mutate the document"
        );
    }
}
