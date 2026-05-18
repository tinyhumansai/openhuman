//! Fixture-driven tests for `dom_snapshot::report_from_snapshot` (issue
//! #1376). The fixture lives at `test_fixtures/dom_snapshot_2026_05.json`
//! and exercises four body-extraction tiers in `find_body`:
//!
//! * **Tier 1** — `<span class="selectable-text">` (legacy WhatsApp Web
//!   shape; some bubbles still render this).
//! * **Tier 2** — `<span dir="ltr">` (current WhatsApp Web shape after
//!   the `selectable-text` class was dropped).
//! * **Tier 3 multi-word (issue #1376 fallback)** — body wrapper with
//!   neither `selectable-text` class nor `dir` hint; only the descendant
//!   text walk + chrome filter recovers the body.
//! * **Tier 3 single-word (regression guard)** — same as tier 3 but with
//!   a one-word body like "ok"; guards against
//!   `looks_like_icon_ligature` false-positives that would silently drop
//!   short plain-text bodies (CodeRabbit issue, fix in #1804).
//!
//! Each row in the fixture stresses exactly one tier so a regression in
//! any tier surfaces as a single failed assertion. The fixture is
//! intentionally synthetic — replace with a captured live snapshot once
//! one is available.
//!
//! Tests use the `pub(crate)` exports `CaptureSnapshot` +
//! `report_from_snapshot` from the parent module so they exercise the
//! full `parse_rows` → `find_body` pipeline.

use super::dom_snapshot::{report_from_snapshot, CaptureSnapshot};

const FIXTURE_2026_05: &str = include_str!("test_fixtures/dom_snapshot_2026_05.json");

fn load_fixture() -> CaptureSnapshot {
    serde_json::from_str(FIXTURE_2026_05)
        .expect("dom_snapshot_2026_05.json must be valid CaptureSnapshot JSON")
}

#[test]
fn parse_rows_finds_four_data_id_rows() {
    let snap = load_fixture();
    let report = report_from_snapshot(&snap);
    assert_eq!(
        report.rows_seen, 4,
        "fixture has four [data-id] rows (tiers 1/2/3-multi/3-single), all should be counted in rows_seen"
    );
}

#[test]
fn capture_pipeline_resolves_active_chat_name() {
    let snap = load_fixture();
    let report = report_from_snapshot(&snap);
    assert!(
        report.active_chat_resolved,
        "fixture has header[data-testid=conversation-header] with text \"Test Chat\""
    );
    assert_eq!(report.active_chat_name.as_deref(), Some("Test Chat"));
}

#[test]
fn find_body_extracts_via_selectable_text_tier1() {
    let snap = load_fixture();
    let report = report_from_snapshot(&snap);
    let row = report
        .rows
        .iter()
        .find(|r| r.msg_id == "msgABC123")
        .expect("tier 1 row (msgABC123) must survive");
    assert_eq!(
        row.body, "hello tier 1",
        "tier 1 row body comes from <span class=selectable-text>"
    );
}

#[test]
fn find_body_extracts_via_dir_attr_tier2() {
    let snap = load_fixture();
    let report = report_from_snapshot(&snap);
    let row = report
        .rows
        .iter()
        .find(|r| r.msg_id == "msgDEF456")
        .expect("tier 2 row (msgDEF456) must survive");
    assert_eq!(
        row.body, "hello tier 2",
        "tier 2 row body comes from <span dir=ltr>"
    );
}

#[test]
fn find_body_extracts_via_descendant_text_tier3_fallback() {
    let snap = load_fixture();
    let report = report_from_snapshot(&snap);
    let row = report.rows.iter().find(|r| r.msg_id == "msgGHI789").expect(
        "tier 3 row (msgGHI789) must survive — body comes from \
             descendant text walk fallback (issue #1376)",
    );
    assert_eq!(
        row.body, "hello tier 3",
        "tier 3 fallback recovers body when neither class nor dir hint is present"
    );
}

#[test]
fn find_body_tier3_does_not_drop_single_word_body() {
    // Regression guard for CodeRabbit finding (PR #1804): the old
    // `looks_like_icon_ligature` treated any lowercase single-token as a
    // ligature, silently dropping "ok", "yes", "hello" etc. via tier-3.
    // This test fails if that regression reappears.
    let snap = load_fixture();
    let report = report_from_snapshot(&snap);
    let row = report.rows.iter().find(|r| r.msg_id == "msgJKL012").expect(
        "tier 3 single-word row (msgJKL012) must survive — \
             looks_like_icon_ligature must not filter plain words like 'ok'",
    );
    assert_eq!(
        row.body, "ok",
        "single-word body 'ok' must not be dropped by looks_like_icon_ligature"
    );
}

#[test]
fn capture_pipeline_extracts_all_four_bodies() {
    let snap = load_fixture();
    let report = report_from_snapshot(&snap);
    assert!(
        report.rows_with_body >= 4,
        "all four tiers should produce non-empty bodies; got rows_with_body={}",
        report.rows_with_body
    );
    assert_eq!(
        report.rows_dropped_no_body, 0,
        "no rows should be dropped when fixture contains body text in every row"
    );
}
