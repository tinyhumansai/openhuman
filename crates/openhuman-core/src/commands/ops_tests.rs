use super::*;
use crate::core::runtime::DispatchCtx;
use serde_json::json;

#[test]
fn builtin_entries_cover_every_documented_slash_command() {
    let entries = builtin_entries();
    let ids: Vec<&str> = entries.iter().map(|e| e.id.as_str()).collect();
    for expected in ["new", "clear", "plan", "build", "goal", "todo", "stop"] {
        assert!(
            ids.contains(&expected),
            "missing builtin {expected}: {ids:?}"
        );
    }
}

#[test]
fn every_builtin_carries_its_own_insert_text() {
    for entry in builtin_entries() {
        assert_eq!(entry.kind, CommandKind::Builtin);
        let insert = entry
            .insert
            .as_deref()
            .expect("builtins always insert text");
        assert!(insert.starts_with('/'), "{insert}");
        assert_eq!(insert.trim_start_matches('/'), entry.id);
    }
}

#[test]
fn entries_from_array_reads_id_name_description() {
    let value = json!({
        "skills": [
            {"id": "s1", "name": "Skill One", "description": "Does a thing"},
            {"id": "s2", "name": "", "description": ""},
        ]
    });
    let entries = entries_from_array(&value, "skills", CommandKind::Skill);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].id, "s1");
    assert_eq!(entries[0].label, "Skill One");
    assert_eq!(entries[0].description, "Does a thing");
    assert!(entries[0].insert.is_none());
    // Blank name falls back to id.
    assert_eq!(entries[1].label, "s2");
    assert_eq!(entries[1].description, "");
}

#[test]
fn entries_from_array_skips_items_with_no_id() {
    let value = json!({ "flows": [ {"name": "no id here"} ] });
    let entries = entries_from_array(&value, "flows", CommandKind::Workflow);
    assert!(entries.is_empty());
}

#[test]
fn entries_from_array_is_empty_for_a_missing_field() {
    let value = json!({ "something_else": [] });
    assert!(entries_from_array(&value, "skills", CommandKind::Skill).is_empty());
}

#[tokio::test]
async fn commands_list_always_includes_every_builtin_even_if_catalogs_fail() {
    // This exercises the real skills/flows registered controllers end to
    // end (no config override) — the important assertion is that whatever
    // they do, every builtin is still present and the call itself never
    // errors.
    let outcome = commands_list(&DispatchCtx::process_default())
        .await
        .expect("commands_list must not fail");
    let ids: Vec<&str> = outcome
        .value
        .commands
        .iter()
        .map(|e| e.id.as_str())
        .collect();
    for expected in ["new", "clear", "plan", "build", "goal", "todo", "stop"] {
        assert!(
            ids.contains(&expected),
            "missing builtin {expected}: {ids:?}"
        );
    }
    let builtin_count = outcome
        .value
        .commands
        .iter()
        .filter(|e| e.kind == CommandKind::Builtin)
        .count();
    assert_eq!(builtin_count, BUILTINS.len());
}
