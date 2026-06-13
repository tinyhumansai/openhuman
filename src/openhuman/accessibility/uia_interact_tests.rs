//! Integration tests for the Windows UI Automation backend of `ax_interact`.
//!
//! These exercise the SAME public helpers the tool uses (`ax_list_elements`,
//! `ax_press_element`, `ax_set_field_value`) — which cfg-dispatch to
//! `uia_interact` on Windows — so they validate the real agent-facing path.
//!
//! Most need a live desktop session and a real app, so they are `#[ignore]` by
//! default. Run them on a Windows machine with:
//!
//!   cargo test --lib uia_interact -- --nocapture --include-ignored
//!
//! `test_uia_list_nonexistent_app` is deterministic (asserts an error) and runs
//! in the normal suite.

#![cfg(all(test, target_os = "windows"))]

use super::{ax_list_elements, ax_list_elements_filtered, ax_press_element, ax_set_field_value};
use std::process::Command;
use std::thread::sleep;
use std::time::Duration;

/// Spawn a launcher exe (e.g. `calc.exe`, `notepad.exe`). Returns whether the
/// spawn itself succeeded; the launched app may appear a moment later.
fn launch(exe: &str) -> bool {
    Command::new(exe).spawn().is_ok()
}

/// Deterministic UI control test (the Windows analogue of the macOS AC/DC test):
/// drive Calculator through `5 + 5 =` purely by element label, then hard-assert
/// the readout shows 10. Calculator is deterministic and always present, so this
/// is a real assertion, not best-effort.
#[test]
#[ignore = "requires a desktop session; launches the Calculator app"]
fn test_uia_calculator_five_plus_five() {
    assert!(launch("calc.exe"), "could not spawn calc.exe");
    sleep(Duration::from_secs(3));

    // 1. List — Calculator should expose its buttons via UIA.
    let elements = ax_list_elements("Calculator").expect("ax_list_elements(Calculator) failed");
    assert!(
        !elements.is_empty(),
        "Calculator exposed no interactive elements — UIA tree empty?"
    );
    println!("[calc] {} interactive elements", elements.len());

    // 2. Press 5, +, 5, = by their (English) UIA Names.
    for label in ["Five", "Plus", "Five", "Equals"] {
        let r = ax_press_element("Calculator", label);
        println!("[calc] press {label}: {r:?}");
        assert!(r.is_ok(), "press '{label}' failed: {r:?}");
        sleep(Duration::from_millis(300));
    }

    // 3. Assert the result readout computed 10.
    sleep(Duration::from_millis(500));
    let readout = ax_list_elements_filtered("Calculator", "Display").unwrap_or_default();
    println!("[calc] readout (filter='Display'): {readout:?}");
    let shows_ten = readout.iter().any(|e| e.label.contains("10"))
        || ax_list_elements("Calculator")
            .unwrap_or_default()
            .iter()
            .any(|e| e.label.contains("10"));
    assert!(
        shows_ten,
        "expected a result element showing 10 after 5 + 5 =; readout={readout:?}"
    );
}

/// Text entry via `set_value` into Notepad's edit control.
///
/// Win11's redesigned Notepad uses a RichEdit control that may not expose the
/// UIA `Value` pattern, whereas classic Notepad does. A `Value`-pattern absence
/// is treated as a documented limitation (best-effort); any other failure is a
/// real test failure.
#[test]
#[ignore = "requires a desktop session; launches Notepad"]
fn test_uia_notepad_set_value() {
    assert!(launch("notepad.exe"), "could not spawn notepad.exe");
    sleep(Duration::from_secs(2));

    let r = ax_set_field_value("Notepad", "", "OpenHuman UIA test");
    println!("[notepad] set_value: {r:?}");
    if let Err(e) = &r {
        // The redesigned Win11 Notepad exposes its editor as a Document/RichEdit
        // that may not support the UIA Value pattern (or any settable text
        // field). That is a documented platform limitation, not a code bug, so
        // treat it as a best-effort skip; classic Notepad / WordPad / ordinary
        // Edit controls still exercise the real write path.
        if e.contains("Value pattern")
            || e.contains("settable")
            || e.contains("no editable text field")
        {
            println!(
                "[notepad] set_value unsupported on this Notepad build \
                 (expected on Win11 RichEdit Notepad): {e}"
            );
            return;
        }
    }
    assert!(r.is_ok(), "set_value failed unexpectedly: {r:?}");
}

/// Deterministic, no-app-needed: a non-existent app must surface an error
/// (either "no window matches" or, in a session-less environment, a UIA-init
/// error — both are `Err` from our wrapper).
#[test]
fn test_uia_list_nonexistent_app() {
    let r = ax_list_elements("OpenHuman_NoSuchApp_ZZZ123");
    assert!(r.is_err(), "expected error for non-existent app, got {r:?}");
    println!(
        "[uia] nonexistent app error (expected): {:?}",
        r.unwrap_err()
    );
}
