//! Tests for POINT tag parsing and coordinate mapping.

use super::*;

fn single_screen() -> Vec<ScreenGeometry> {
    vec![ScreenGeometry {
        index: 0,
        x: 0.0,
        y: 0.0,
        width: 1920.0,
        height: 1080.0,
    }]
}

fn dual_screens() -> Vec<ScreenGeometry> {
    vec![
        ScreenGeometry {
            index: 0,
            x: 0.0,
            y: 0.0,
            width: 1920.0,
            height: 1080.0,
        },
        ScreenGeometry {
            index: 1,
            x: 1920.0,
            y: 0.0,
            width: 2560.0,
            height: 1440.0,
        },
    ]
}

// ── Basic parsing ─────────────────────────────────────────────────────

#[test]
fn parse_single_point_tag() {
    let text = "Click the button [POINT:100,200:Save Button:screen0] to save.";
    let result = parse_and_map(text, &single_screen());

    assert_eq!(result.targets.len(), 1);
    assert_eq!(result.targets[0].x, 100.0);
    assert_eq!(result.targets[0].y, 200.0);
    assert_eq!(result.targets[0].label, "Save Button");
    assert_eq!(result.targets[0].screen_index, 0);
    assert_eq!(result.clean_text, "Click the button  to save.");
}

#[test]
fn parse_multiple_point_tags() {
    let text = "First [POINT:10,20:A:screen0] then [POINT:30,40:B:screen1] done.";
    let result = parse_and_map(text, &dual_screens());

    assert_eq!(result.targets.len(), 2);
    assert_eq!(result.targets[0].label, "A");
    assert_eq!(result.targets[1].label, "B");
}

#[test]
fn parse_no_point_tags() {
    let text = "No pointing needed here.";
    let result = parse_and_map(text, &single_screen());

    assert!(result.targets.is_empty());
    assert_eq!(result.clean_text, "No pointing needed here.");
}

#[test]
fn parse_decimal_coordinates() {
    let text = "[POINT:100.5,200.75:Pin:screen0]";
    let result = parse_and_map(text, &single_screen());

    assert_eq!(result.targets[0].x, 100.5);
    assert_eq!(result.targets[0].y, 200.75);
}

#[test]
fn parse_negative_coordinates_clamped() {
    let text = "[POINT:-50,-100:Off-screen:screen0]";
    let result = parse_and_map(text, &single_screen());

    assert_eq!(result.targets[0].absolute_x, 0.0);
    assert_eq!(result.targets[0].absolute_y, 0.0);
}

// ── Multi-monitor mapping ─────────────────────────────────────────────

#[test]
fn map_to_primary_screen() {
    let text = "[POINT:500,300:Target:screen0]";
    let result = parse_and_map(text, &dual_screens());

    assert_eq!(result.targets[0].absolute_x, 500.0);
    assert_eq!(result.targets[0].absolute_y, 300.0);
}

#[test]
fn map_to_secondary_screen() {
    let text = "[POINT:500,300:Target:screen1]";
    let result = parse_and_map(text, &dual_screens());

    // screen1 starts at x=1920
    assert_eq!(result.targets[0].absolute_x, 2420.0);
    assert_eq!(result.targets[0].absolute_y, 300.0);
}

#[test]
fn screen_index_out_of_range_falls_back_to_primary() {
    let text = "[POINT:100,200:Target:screen5]";
    let result = parse_and_map(text, &single_screen());

    // Falls back to screen 0
    assert_eq!(result.targets[0].absolute_x, 100.0);
    assert_eq!(result.targets[0].absolute_y, 200.0);
}

#[test]
fn coordinates_clamped_to_screen_bounds() {
    let text = "[POINT:5000,3000:Far:screen0]";
    let result = parse_and_map(text, &single_screen());

    assert_eq!(result.targets[0].absolute_x, 1920.0);
    assert_eq!(result.targets[0].absolute_y, 1080.0);
}

#[test]
fn empty_screens_returns_raw_coordinates() {
    let text = "[POINT:100,200:Target:screen0]";
    let result = parse_and_map(text, &[]);

    assert_eq!(result.targets[0].absolute_x, 100.0);
    assert_eq!(result.targets[0].absolute_y, 200.0);
}

// ── Malformed tags ────────────────────────────────────────────────────

#[test]
fn malformed_tag_not_parsed() {
    let text = "[POINT:abc,def:Bad:screen0] and [POINT:100:Missing:screen0]";
    let result = parse_and_map(text, &single_screen());

    // Neither matches the regex
    assert!(result.targets.is_empty());
}

#[test]
fn partial_tag_not_parsed() {
    let text = "[POINT:100,200:No Screen] and POINT:100,200:bare:screen0]";
    let result = parse_and_map(text, &single_screen());

    assert!(result.targets.is_empty());
}

// ── Clean text ────────────────────────────────────────────────────────

#[test]
fn clean_text_strips_all_tags() {
    let text = "Start [POINT:0,0:A:screen0] middle [POINT:0,0:B:screen0] end";
    let result = parse_and_map(text, &single_screen());

    assert_eq!(result.clean_text, "Start  middle  end");
}

#[test]
fn clean_text_trims_whitespace() {
    let text = "  [POINT:0,0:A:screen0]  ";
    let result = parse_and_map(text, &single_screen());

    assert_eq!(result.clean_text, "");
}
