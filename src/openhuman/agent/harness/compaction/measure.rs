//! Deterministic token-savings harness for compaction (the `[agent_cost]`
//! A/B, measured at the compaction boundary).
//!
//! A live A/B runs the agent against the backend with `OPENHUMAN_COMPACTION=0`
//! vs on and diffs the `[agent_cost] … tokens_in=…` lines. That needs LLM
//! credentials + a real workspace, so it's the operator's job (commands in
//! `compaction-plan.md`). What we *can* pin down reproducibly is the input-side
//! reduction those `tokens_in` deltas come from: run representative tool
//! outputs through [`super::compact_tool_output`] and measure the token delta
//! with the **same estimator the budget/cost path uses**
//! ([`super::super::token_budget::estimate_tokens`]).
//!
//! These tests double as a regression guard: if a future change regresses the
//! savings on a fixture, they fail. Run with output:
//!
//! ```text
//! cargo test -p openhuman --lib compaction::measure -- --nocapture
//! ```

#![cfg(test)]

use super::super::token_budget::estimate_tokens;
use super::compact_tool_output;
use std::fmt::Write as _;

/// A single A/B sample: token counts before/after compaction for one fixture.
struct Sample {
    label: &'static str,
    tool: &'static str,
    tokens_before: usize,
    tokens_after: usize,
}

impl Sample {
    fn run(label: &'static str, tool: &'static str, raw: String) -> Self {
        let before = estimate_tokens(&raw);
        let compacted = compact_tool_output(raw, tool, true);
        let after = estimate_tokens(&compacted);
        Sample {
            label,
            tool,
            tokens_before: before,
            tokens_after: after,
        }
    }

    fn saved_pct(&self) -> f64 {
        if self.tokens_before == 0 {
            0.0
        } else {
            100.0 * (1.0 - self.tokens_after as f64 / self.tokens_before as f64)
        }
    }
}

// ── Representative fixtures (the loud tool families from the plan) ──────────
// Note: grep/search is intentionally not compacted, so it is not measured here.

fn cargo_test_log_fixture() -> String {
    let mut s = String::new();
    for i in 0..300 {
        let _ = writeln!(s, "   Compiling dependency_crate_{i} v0.{i}.0");
    }
    for i in 0..40 {
        let _ = writeln!(
            s,
            "warning: unused variable `tmp` at src/x.rs (occurrence {i})"
        );
    }
    let _ = writeln!(s, "error[E0382]: borrow of moved value `config`");
    let _ = writeln!(s, "  --> src/server/boot.rs:142:18");
    let _ = writeln!(s, "error: aborting due to previous error");
    let _ = writeln!(s, "test result: FAILED. 87 passed; 1 failed; 0 ignored");
    s
}

fn json_list_fixture() -> String {
    let mut rows = Vec::new();
    for i in 0..150 {
        rows.push(format!(
            r#"{{"id":{i},"name":"user_{i}","email":"user{i}@example.com","status":"active","role":"member"}}"#
        ));
    }
    format!("[{}]", rows.join(","))
}

fn diff_fixture() -> String {
    let mut s = String::from("diff --git a/src/big.rs b/src/big.rs\n@@ -1,80 +1,82 @@\n");
    for i in 0..40 {
        let _ = writeln!(s, " unchanged context line {i} that the diff carries along");
    }
    let _ = writeln!(s, "-    let x = old_implementation();");
    let _ = writeln!(s, "+    let x = new_implementation(with_args);");
    for i in 0..40 {
        let _ = writeln!(s, " more unchanged context line {i} after the change");
    }
    s
}

#[test]
fn ab_token_savings_report() {
    let samples = vec![
        Sample::run("cargo test failure", "run_tests", cargo_test_log_fixture()),
        Sample::run("JSON list (150 rows)", "list_records", json_list_fixture()),
        Sample::run("git diff (large context)", "read_diff", diff_fixture()),
    ];

    let mut total_before = 0usize;
    let mut total_after = 0usize;
    println!("\n[compaction A/B] token_in savings at the compaction boundary (estimate_tokens):");
    println!(
        "  {:<26} {:>10} {:>10} {:>9}",
        "workload", "before", "after", "saved"
    );
    for s in &samples {
        total_before += s.tokens_before;
        total_after += s.tokens_after;
        println!(
            "  {:<26} {:>10} {:>10} {:>8.0}%   (tool={})",
            s.label,
            s.tokens_before,
            s.tokens_after,
            s.saved_pct(),
            s.tool
        );
        // Each loud-family fixture must save meaningfully — regression guard.
        assert!(
            s.saved_pct() >= 30.0,
            "{} only saved {:.0}%",
            s.label,
            s.saved_pct()
        );
    }
    let overall = 100.0 * (1.0 - total_after as f64 / total_before as f64);
    println!(
        "  {:<26} {:>10} {:>10} {:>8.0}%",
        "TOTAL", total_before, total_after, overall
    );
    assert!(overall >= 50.0, "overall savings only {overall:.0}%");
}

#[test]
fn disabled_yields_zero_savings() {
    // Sanity: with the kill-switch off, the harness sees no reduction — this is
    // the control arm of the A/B. (Uses a fixture that *would* compact when on.)
    let raw = cargo_test_log_fixture();
    let before = estimate_tokens(&raw);
    let after = estimate_tokens(&compact_tool_output(raw, "run_tests", false));
    assert_eq!(before, after);
}
