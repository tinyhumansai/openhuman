//! Integration tests: iterate vendored fixtures and assert parity with
//! expected output.
//!
//! Each `*.fixture.json` in the `fixtures/` directory has the shape:
//! ```json
//! {
//!   "description": "...",
//!   "input": { ... ToolExecutionInput ... },
//!   "expectedOutput": "..."
//! }
//! ```
//!
//! The test loads all rules (builtin-only) and runs `reduce_execution_with_rules`,
//! then asserts that `result.inline_text == expectedOutput`.
//!
//! If the Rust port produces different output from a fixture (e.g. due to
//! `Intl.Segmenter` vs `unicode-segmentation` boundary differences), the
//! discrepancy is documented in `KNOWN_DRIFT.md` and the fixture is listed
//! in `KNOWN_DRIFT_FIXTURES` below to be skipped in CI.

use openhuman_core::openhuman::tokenjuice::{
    reduce::reduce_execution_with_rules,
    rules::load_builtin_rules,
    types::{ReduceOptions, RuleFixture},
};

/// Fixture file names (relative to `fixtures/`) that are known to drift from
/// upstream TS output.  Add entries here with a comment explaining why.
const KNOWN_DRIFT_FIXTURES: &[&str] = &[
    // None currently — add entries if parity tests fail due to
    // Intl.Segmenter vs unicode-segmentation differences.
];

fn fixtures_dir() -> std::path::PathBuf {
    // CARGO_MANIFEST_DIR points to the workspace root; navigate to the fixtures
    let manifest = std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR not set");
    std::path::PathBuf::from(manifest)
        .join("src/openhuman/tokenjuice/tests/fixtures")
}

#[test]
fn all_fixtures_pass() {
    let dir = fixtures_dir();
    if !dir.is_dir() {
        eprintln!("fixtures directory not found: {}", dir.display());
        return;
    }

    let rules = load_builtin_rules();
    let mut passed = 0usize;
    let mut skipped = 0usize;
    let mut failed = 0usize;

    let mut entries: Vec<_> = std::fs::read_dir(&dir)
        .expect("read fixtures dir")
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_string_lossy()
                .ends_with(".fixture.json")
        })
        .collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let path = entry.path();
        let name = path.file_name().unwrap().to_string_lossy().to_string();

        if KNOWN_DRIFT_FIXTURES.iter().any(|&s| name == s) {
            eprintln!("[SKIP] {} (known drift)", name);
            skipped += 1;
            continue;
        }

        let json = std::fs::read_to_string(&path).expect("read fixture");
        let fixture: RuleFixture = match serde_json::from_str(&json) {
            Ok(f) => f,
            Err(e) => {
                eprintln!("[FAIL] {} — JSON parse error: {}", name, e);
                failed += 1;
                continue;
            }
        };

        let opts = fixture.options.unwrap_or_default();
        let result = reduce_execution_with_rules(fixture.input, &rules, &opts);

        if result.inline_text.trim() == fixture.expected_output.trim() {
            passed += 1;
        } else {
            eprintln!("[FAIL] {}", name);
            if let Some(desc) = &fixture.description {
                eprintln!("  description: {}", desc);
            }
            eprintln!("  expected: {:?}", fixture.expected_output.trim());
            eprintln!("  actual:   {:?}", result.inline_text.trim());
            failed += 1;
        }
    }

    eprintln!(
        "\nfixture summary: {} passed, {} skipped, {} failed",
        passed, skipped, failed
    );

    assert_eq!(
        failed, 0,
        "{} fixture(s) failed — see output above",
        failed
    );
}
