use super::*;
use std::fs;
use tempfile::TempDir;

fn write_file(path: &std::path::Path, body: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, body).unwrap();
}

#[test]
fn fresh_workspace_writes_marker_with_no_moves() {
    let dir = TempDir::new().unwrap();
    let outcome = migrate_session_layout_if_needed(dir.path()).unwrap();
    assert!(!outcome.already_done);
    assert_eq!(outcome.jsonl_moved, 0);
    assert_eq!(outcome.md_moved, 0);
    assert!(marker_path_for(dir.path()).exists());
}

#[test]
fn second_run_is_a_noop() {
    let dir = TempDir::new().unwrap();
    let _first = migrate_session_layout_if_needed(dir.path()).unwrap();
    let second = migrate_session_layout_if_needed(dir.path()).unwrap();
    assert!(second.already_done);
    assert_eq!(second.jsonl_moved, 0);
    assert_eq!(second.warnings.len(), 0);
}

#[test]
fn moves_legacy_jsonl_files_up_to_flat_session_raw() {
    let dir = TempDir::new().unwrap();
    let ws = dir.path();
    let legacy_a = ws.join("session_raw").join("01052026");
    let legacy_b = ws.join("session_raw").join("02052026");
    write_file(&legacy_a.join("1714000000_main.jsonl"), "a");
    write_file(&legacy_a.join("1714000001_welcome.jsonl"), "b");
    write_file(&legacy_b.join("1714999999_orchestrator.jsonl"), "c");

    let outcome = migrate_session_layout_if_needed(ws).unwrap();
    assert_eq!(outcome.jsonl_moved, 3);
    assert_eq!(outcome.legacy_dirs_pruned, 2);

    let raw_root = ws.join("session_raw");
    assert!(raw_root.join("1714000000_main.jsonl").exists());
    assert!(raw_root.join("1714000001_welcome.jsonl").exists());
    assert!(raw_root.join("1714999999_orchestrator.jsonl").exists());
    // Empty legacy date dirs should have been pruned.
    assert!(!legacy_a.exists(), "legacy date dir should be removed");
    assert!(!legacy_b.exists(), "legacy date dir should be removed");
}

#[test]
fn jsonl_destination_collision_is_skipped_with_warning() {
    // If a flat `session_raw/{stem}.jsonl` already exists for the
    // same stem we don't overwrite — the flat copy is authoritative
    // (the user may have already started a fresh session with the
    // same key after a clock reset). Surface a warning instead.
    let dir = TempDir::new().unwrap();
    let ws = dir.path();
    let raw_root = ws.join("session_raw");
    write_file(&raw_root.join("1714000000_main.jsonl"), "new");
    write_file(
        &raw_root.join("01052026").join("1714000000_main.jsonl"),
        "old",
    );

    let outcome = migrate_session_layout_if_needed(ws).unwrap();
    assert_eq!(outcome.jsonl_moved, 0);
    assert_eq!(outcome.jsonl_skipped, 1);
    assert!(outcome
        .warnings
        .iter()
        .any(|w| w.contains("already exists")));
    // Both files still exist — nothing was overwritten.
    assert_eq!(
        fs::read_to_string(raw_root.join("1714000000_main.jsonl")).unwrap(),
        "new"
    );
    assert_eq!(
        fs::read_to_string(raw_root.join("01052026").join("1714000000_main.jsonl")).unwrap(),
        "old"
    );
}

#[test]
fn renames_md_ddmmyyyy_dirs_to_iso() {
    let dir = TempDir::new().unwrap();
    let ws = dir.path();
    let legacy_md = ws.join("sessions").join("01052026");
    write_file(&legacy_md.join("main_0.md"), "x");
    write_file(&legacy_md.join("main_1.md"), "y");

    let outcome = migrate_session_layout_if_needed(ws).unwrap();
    assert_eq!(outcome.md_moved, 1, "one rename of the dir as a whole");
    let iso = ws.join("sessions").join("2026_05_01");
    assert!(iso.is_dir());
    assert!(iso.join("main_0.md").exists());
    assert!(iso.join("main_1.md").exists());
    assert!(
        !legacy_md.exists(),
        "DDMMYYYY md dir should be gone after rename"
    );
}

#[test]
fn merges_md_when_iso_dir_already_exists() {
    let dir = TempDir::new().unwrap();
    let ws = dir.path();
    // Both layouts coexist for the same calendar date — e.g. user
    // ran a hand-edited build that produced ISO dirs alongside the
    // legacy DDMMYYYY ones.
    let legacy = ws.join("sessions").join("01052026");
    let iso = ws.join("sessions").join("2026_05_01");
    write_file(&legacy.join("main_0.md"), "legacy");
    write_file(&legacy.join("main_1.md"), "legacy");
    write_file(&iso.join("main_1.md"), "newer");

    let outcome = migrate_session_layout_if_needed(ws).unwrap();
    // main_0.md moves over (no collision); main_1.md collides and is
    // skipped without overwriting the newer copy.
    assert_eq!(outcome.md_moved, 1);
    assert_eq!(outcome.md_skipped, 1);
    assert_eq!(fs::read_to_string(iso.join("main_0.md")).unwrap(), "legacy");
    assert_eq!(fs::read_to_string(iso.join("main_1.md")).unwrap(), "newer");
}

#[test]
fn ignores_non_date_subdirectories_in_session_raw() {
    // Defensive: a user (or some other tool) might have created a
    // sibling dir under session_raw/. We must not touch it — only
    // 8-digit names are recognised as legacy date dirs.
    let dir = TempDir::new().unwrap();
    let ws = dir.path();
    let weird = ws.join("session_raw").join("my_notes");
    write_file(&weird.join("random.jsonl"), "keep me");

    let outcome = migrate_session_layout_if_needed(ws).unwrap();
    assert_eq!(outcome.jsonl_moved, 0);
    assert!(weird.is_dir(), "non-date subdir must be left alone");
    assert!(weird.join("random.jsonl").exists());
}

#[test]
fn ddmmyyyy_to_iso_handles_boundary_dates() {
    assert_eq!(
        ddmmyyyy_to_yyyy_mm_dd("01012026").as_deref(),
        Some("2026_01_01")
    );
    assert_eq!(
        ddmmyyyy_to_yyyy_mm_dd("31122099").as_deref(),
        Some("2099_12_31")
    );
    assert!(ddmmyyyy_to_yyyy_mm_dd("abc12345").is_none());
    assert!(ddmmyyyy_to_yyyy_mm_dd("1234567").is_none(), "7 digits");
    assert!(ddmmyyyy_to_yyyy_mm_dd("123456789").is_none(), "9 digits");
}

#[test]
fn marker_persists_run_metadata() {
    let dir = TempDir::new().unwrap();
    let ws = dir.path();
    let legacy = ws.join("session_raw").join("01052026");
    write_file(&legacy.join("1714000000_main.jsonl"), "a");

    migrate_session_layout_if_needed(ws).unwrap();
    let marker = fs::read_to_string(marker_path_for(ws)).unwrap();
    assert!(marker.contains("jsonl_moved: 1"));
    assert!(marker.contains("openhuman session_layout migration v1"));
}
