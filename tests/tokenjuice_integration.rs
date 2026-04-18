//! Integration tests for the TokenJuice module.
//!
//! Iterates vendored `*.fixture.json` files under
//! `src/openhuman/tokenjuice/tests/fixtures/` and asserts that
//! `reduce_execution_with_rules` produces the expected output.

use openhuman_core::openhuman::tokenjuice::{
    reduce::reduce_execution_with_rules, rules::load_builtin_rules, types::RuleFixture,
};

/// Fixture names that are known to produce different output from the upstream
/// TypeScript — typically due to `Intl.Segmenter` vs `unicode-segmentation`
/// grapheme-boundary differences.  See `KNOWN_DRIFT.md` for rationale.
const KNOWN_DRIFT_FIXTURES: &[&str] = &[
    // None currently.
];

fn fixtures_dir() -> std::path::PathBuf {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    std::path::PathBuf::from(manifest).join("src/openhuman/tokenjuice/tests/fixtures")
}

#[test]
fn vendored_fixtures_match_expected_output() {
    let dir = fixtures_dir();
    assert!(
        dir.is_dir(),
        "fixtures directory not found: {}",
        dir.display()
    );

    let rules = load_builtin_rules();
    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read fixtures dir")
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".fixture.json"))
        .collect();
    entries.sort_by_key(|e| e.file_name());

    let mut passed = 0usize;
    let mut skipped = 0usize;
    let mut failures: Vec<String> = Vec::new();

    for entry in &entries {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        if KNOWN_DRIFT_FIXTURES.iter().any(|&s| s == name) {
            eprintln!("[SKIP] {} (known drift)", name);
            skipped += 1;
            continue;
        }

        let json = std::fs::read_to_string(&path).expect("read fixture file");
        let fixture: RuleFixture = serde_json::from_str(&json)
            .unwrap_or_else(|e| panic!("JSON parse error in {}: {}", name, e));

        let opts = fixture.options.clone().unwrap_or_default();
        let result = reduce_execution_with_rules(fixture.input.clone(), &rules, &opts);

        if result.inline_text.trim() == fixture.expected_output.trim() {
            passed += 1;
        } else {
            let msg = format!(
                "[FAIL] {}\n  desc:     {}\n  expected: {:?}\n  actual:   {:?}",
                name,
                fixture.description.as_deref().unwrap_or("(none)"),
                fixture.expected_output.trim(),
                result.inline_text.trim()
            );
            eprintln!("{}", msg);
            failures.push(name);
        }
    }

    eprintln!(
        "\ntokenjuice integration: {} passed, {} skipped, {} failed",
        passed,
        skipped,
        failures.len()
    );

    assert!(
        failures.is_empty(),
        "{} fixture(s) failed: {}",
        failures.len(),
        failures.join(", ")
    );
}
