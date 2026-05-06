//! Tests for the subconscious → conversation bridge.
//!
//! Most tests use a tempdir as the workspace so the JSONL store writes
//! land somewhere disposable. We exercise: thread idempotency, message
//! body rendering with/without `proposed_action`, and the
//! `extra_metadata` shape.

use super::*;
use crate::openhuman::subconscious::reflection::{
    hydrate_draft, Disposition, ReflectionDraft, ReflectionKind,
};
use tempfile::TempDir;

fn refl(disposition: Disposition, proposed: Option<&str>) -> Reflection {
    let draft = ReflectionDraft {
        kind: ReflectionKind::Opportunity,
        body: "User mentioned founders dinner".into(),
        disposition,
        proposed_action: proposed.map(String::from),
        source_refs: vec!["entity:dinner".into(), "summary:abc".into()],
    };
    hydrate_draft(draft, "refl-1".into(), 1.0)
}

#[test]
fn render_body_without_action_is_plain() {
    let r = refl(Disposition::Notify, None);
    let body = render_message_body(&r);
    assert_eq!(body, "User mentioned founders dinner");
}

#[test]
fn render_body_with_action_appends_proposed_action() {
    let r = refl(Disposition::Notify, Some("Draft an invite list"));
    let body = render_message_body(&r);
    assert!(body.contains("User mentioned founders dinner"));
    assert!(body.contains("_Proposed action_:"));
    assert!(body.contains("Draft an invite list"));
}

#[test]
fn render_body_ignores_blank_action() {
    let r = refl(Disposition::Notify, Some("   "));
    let body = render_message_body(&r);
    assert_eq!(body, "User mentioned founders dinner");
}

#[test]
fn extra_metadata_carries_all_fields() {
    let r = refl(Disposition::Notify, Some("Pull invites"));
    let meta = build_extra_metadata(&r);
    assert_eq!(meta["reflection_id"], "refl-1");
    assert_eq!(meta["kind"], "opportunity");
    assert_eq!(meta["disposition"], "notify");
    assert_eq!(meta["proposed_action"], "Pull invites");
    assert!(meta["source_refs"].is_array());
    assert_eq!(meta["source_refs"].as_array().unwrap().len(), 2);
}

#[test]
fn ensure_thread_is_idempotent() {
    let tmp = TempDir::new().expect("tempdir");
    let workspace = tmp.path().to_path_buf();
    let t1 = ensure_subconscious_thread(workspace.clone(), "2026-05-07T00:00:00Z".into())
        .expect("ensure 1");
    let t2 = ensure_subconscious_thread(workspace, "2026-05-07T01:00:00Z".into()).expect("ensure 2");
    assert_eq!(t1.id, t2.id);
    assert_eq!(t1.id, SUBCONSCIOUS_THREAD_ID);
    assert_eq!(t1.title, SUBCONSCIOUS_THREAD_TITLE);
    assert!(t1.labels.iter().any(|l| l == "subconscious"));
}

#[test]
fn post_reflection_rejects_observe_disposition() {
    let tmp = TempDir::new().expect("tempdir");
    let r = refl(Disposition::Observe, Some("Take a look"));
    let err = post_reflection(tmp.path().to_path_buf(), &r).expect_err("should refuse");
    assert!(err.contains("Notify"));
}

#[test]
fn post_reflection_appends_message_with_metadata() {
    let tmp = TempDir::new().expect("tempdir");
    let workspace = tmp.path().to_path_buf();
    ensure_subconscious_thread(workspace.clone(), "2026-05-07T00:00:00Z".into())
        .expect("ensure thread");
    let r = refl(Disposition::Notify, Some("Draft an invite list"));
    let msg = post_reflection(workspace, &r).expect("post");
    assert_eq!(msg.sender, "assistant");
    assert!(msg.content.contains("founders dinner"));
    assert_eq!(msg.extra_metadata["reflection_id"], "refl-1");
    assert_eq!(msg.extra_metadata["kind"], "opportunity");
}
