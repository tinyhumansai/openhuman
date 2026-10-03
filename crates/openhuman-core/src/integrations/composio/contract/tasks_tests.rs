//! Tests for the task-fetch envelope — the pure-data half.
//!
//! The provider mappings that populate a [`super::NormalizedTask`] live in the
//! engine crate with the providers that own them. What is pinned here is the
//! envelope itself: the wire casing the task-source UI reads, the enum tags
//! that end up in a card's `source_metadata`, and the unset-cap rule that would
//! otherwise read as "fetch nothing".

#![allow(clippy::expect_used, clippy::unwrap_used, clippy::panic)]

use super::{GithubFetchMode, NormalizedTask, TaskContainer, TaskFetchFilter, TaskKind};

#[test]
fn every_task_kind_tag_matches_its_serde_form() {
    for kind in [TaskKind::Generic, TaskKind::Issue, TaskKind::PullRequest] {
        let json = serde_json::to_string(&kind).expect("serialize");
        assert_eq!(
            json,
            format!("\"{}\"", kind.as_str()),
            "as_str and the serde form disagree for {kind:?}"
        );
    }
}

#[test]
fn task_kind_tags_are_the_stable_strings() {
    assert_eq!(TaskKind::Generic.as_str(), "generic");
    assert_eq!(TaskKind::Issue.as_str(), "issue");
    assert_eq!(TaskKind::PullRequest.as_str(), "pull_request");
}

#[test]
fn an_undifferentiated_task_defaults_to_generic() {
    assert_eq!(TaskKind::default(), TaskKind::Generic);
    assert_eq!(NormalizedTask::default().kind, TaskKind::Generic);
}

#[test]
fn the_github_fetch_mode_defaults_to_auto() {
    // `Auto` is the safe default: a shipped user with no `gh` on `PATH` still
    // reaches GitHub through the connected Composio account.
    assert_eq!(GithubFetchMode::default(), GithubFetchMode::Auto);
    assert_eq!(
        TaskFetchFilter::default().github_fetch_mode,
        GithubFetchMode::Auto
    );
}

#[test]
fn every_github_fetch_mode_round_trips_through_its_snake_case_tag() {
    let cases = [
        (GithubFetchMode::Auto, "\"auto\""),
        (GithubFetchMode::Composio, "\"composio\""),
        (GithubFetchMode::Local, "\"local\""),
    ];
    for (mode, wire) in cases {
        let json = serde_json::to_string(&mode).expect("serialize");
        assert_eq!(json, wire, "wire form changed for {mode:?}");
        let back: GithubFetchMode = serde_json::from_str(&json).expect("deserialize");
        assert_eq!(back, mode);
    }
}

#[test]
fn a_normalized_task_serialises_camel_case_for_the_ui() {
    let task = NormalizedTask {
        external_id: "42".into(),
        source_id: "src-1".into(),
        provider: "github".into(),
        kind: TaskKind::PullRequest,
        title: "Fix the thing".into(),
        updated_at: Some("2026-08-25T10:00:00Z".into()),
        ..NormalizedTask::default()
    };
    let json = serde_json::to_value(&task).expect("serialize to value");
    let object = json.as_object().expect("task serialises as an object");

    assert!(object.contains_key("externalId"));
    assert!(object.contains_key("sourceId"));
    assert!(object.contains_key("updatedAt"));
    assert_eq!(object["kind"], "pull_request");
    // Absent optionals are skipped rather than emitted as null, so the UI can
    // distinguish "the provider had nothing" from "the field is new".
    assert!(!object.contains_key("body"));
    assert!(!object.contains_key("url"));
}

#[test]
fn a_normalized_task_round_trips() {
    let task = NormalizedTask {
        external_id: "7".into(),
        provider: "linear".into(),
        title: "Ship it".into(),
        labels: vec!["p1".into()],
        raw: serde_json::json!({ "id": 7 }),
        ..NormalizedTask::default()
    };
    let json = serde_json::to_string(&task).expect("serialize");
    let back: NormalizedTask = serde_json::from_str(&json).expect("deserialize");
    assert_eq!(back, task);
}

#[test]
fn a_normalized_task_decodes_from_the_minimum_an_older_peer_wrote() {
    // Every field beyond the three required ones carries `#[serde(default)]`,
    // so a peer built before any of them existed still decodes.
    let back: NormalizedTask =
        serde_json::from_str(r#"{"externalId":"1","provider":"notion","title":"t"}"#)
            .expect("deserialize minimal");
    assert_eq!(back.external_id, "1");
    assert_eq!(back.source_id, "");
    assert_eq!(back.kind, TaskKind::Generic);
    assert!(back.labels.is_empty());
}

#[test]
fn an_unset_cap_becomes_a_safe_bound_rather_than_zero() {
    assert_eq!(TaskFetchFilter::default().effective_max(), 25);
}

#[test]
fn an_explicit_cap_is_honoured() {
    let filter = TaskFetchFilter {
        max: 3,
        ..TaskFetchFilter::default()
    };
    assert_eq!(filter.effective_max(), 3);
}

#[test]
fn a_task_container_serialises_the_picker_shape() {
    let container = TaskContainer {
        id: "db-1".into(),
        title: "Roadmap".into(),
    };
    let json = serde_json::to_value(&container).expect("serialize to value");
    assert_eq!(json["id"], "db-1");
    assert_eq!(json["title"], "Roadmap");
}
