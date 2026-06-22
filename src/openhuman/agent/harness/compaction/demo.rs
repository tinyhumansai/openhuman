//! Human-eyeball demo of compaction on dummy data.
//!
//! Unlike [`super::measure`] (which asserts token deltas), this prints the
//! actual BEFORE → AFTER text for each content type so you can see exactly
//! what the model would receive. Run it with output:
//!
//! ```text
//! cargo test -p openhuman --lib compaction::demo -- --nocapture
//! ```

#![cfg(test)]

use super::super::token_budget::estimate_tokens;
use super::{compact_tool_output, store};
use std::fmt::Write as _;

/// Print a labelled before/after block. Input is shown head+tail so the
/// terminal stays readable; the compacted output is shown in full.
fn show(title: &str, tool: &str, raw: &str) -> String {
    let before = estimate_tokens(raw);
    let out = compact_tool_output(raw.to_string(), tool, true);
    let after = estimate_tokens(&out);
    let pct = if before == 0 {
        0.0
    } else {
        100.0 * (1.0 - after as f64 / before as f64)
    };

    println!("\n══════════════════════════════════════════════════════════════");
    println!("▶ {title}   (tool={tool})");
    println!(
        "  {} bytes / ~{} tok  →  {} bytes / ~{} tok   ({:.0}% saved)",
        raw.len(),
        before,
        out.len(),
        after,
        pct
    );
    println!("─── INPUT (first 8 + last 2 lines) ──────────────────────────");
    print_head_tail(raw, 8, 2);
    println!("─── COMPACTED (what the model sees) ─────────────────────────");
    println!("{out}");
    out
}

fn print_head_tail(text: &str, head: usize, tail: usize) {
    let lines: Vec<&str> = text.lines().collect();
    if lines.len() <= head + tail {
        for l in &lines {
            println!("{l}");
        }
        return;
    }
    for l in &lines[..head] {
        println!("{l}");
    }
    println!("        … {} lines …", lines.len() - head - tail);
    for l in &lines[lines.len() - tail..] {
        println!("{l}");
    }
}

#[test]
fn demo_all_content_types() {
    // 1 ── grep / code search is INTENTIONALLY NOT compacted (completeness
    //    tool). Demonstrate that it passes through untouched even when large.
    let mut grep = String::from("48 match(es); scanned 4 file(s)\n");
    for f in 0..4 {
        for i in 1..=12 {
            let _ = writeln!(
                grep,
                "src/feature_{f}/service.rs:{i}:    let outcome = dispatch_request(&context, request_payload_{i})?;"
            );
        }
    }
    let grep_out = compact_tool_output(grep.clone(), "grep", true);
    println!("\n══════════════════════════════════════════════════════════════");
    println!("▶ Code search (grep) — intentionally NOT compacted");
    println!(
        "  {} bytes → {} bytes   (unchanged: {})",
        grep.len(),
        grep_out.len(),
        grep_out == grep
    );

    // 2 ── build/test log: noise + warnings + a real error + summary.
    let mut log = String::new();
    for i in 0..120 {
        let _ = writeln!(log, "   Compiling some_dependency_{i} v1.{i}.0");
    }
    for i in 0..30 {
        let _ = writeln!(
            log,
            "warning: unused variable `scratch` (occurrence {i}) at src/util.rs"
        );
    }
    let _ = writeln!(log, "error[E0382]: borrow of moved value `session`");
    let _ = writeln!(log, "   --> src/agent/loop.rs:88:21");
    let _ = writeln!(log, "    |");
    let _ = writeln!(
        log,
        "    = note: move occurs because `session` has type `Session`"
    );
    let _ = writeln!(log, "error: aborting due to previous error");
    let _ = writeln!(log, "test result: FAILED. 142 passed; 1 failed; 3 ignored");
    show("Build/test log (run_tests)", "run_tests", &log);

    // 3 ── git diff: small change wrapped in lots of unchanged context.
    //    (Fixtures are sized above MIN_BYTES_TO_COMPRESS = 2048 so the
    //    compressors actually engage — smaller outputs pass through untouched.)
    let mut diff =
        String::from("diff --git a/src/router.rs b/src/router.rs\n@@ -10,84 +10,85 @@\n");
    for i in 0..40 {
        let _ = writeln!(
            diff,
            " // unchanged surrounding context line {i} carried along by the diff"
        );
    }
    let _ = writeln!(diff, "-    route.register(\"/old\", old_handler);");
    let _ = writeln!(diff, "+    route.register(\"/new\", new_handler);");
    let _ = writeln!(diff, "+    route.register(\"/extra\", extra_handler);");
    for i in 0..40 {
        let _ = writeln!(
            diff,
            " // trailing unchanged context line {i} after the change"
        );
    }
    show("Git diff (read_diff)", "read_diff", &diff);

    // 4 ── JSON list, small enough to stay lossless (under the row-drop
    //    threshold) but big enough to clear the byte floor → lossless table.
    let mut small = Vec::new();
    for i in 0..30 {
        small.push(format!(
            r#"{{"id":{i},"name":"widget_{i}","status":"in_stock","warehouse":"east-1","sku":"WH-{i:04}"}}"#
        ));
    }
    show(
        "JSON list — 30 rows (lossless table)",
        "list_inventory",
        &format!("[{}]", small.join(",")),
    );

    // 5 ── JSON list, large (row-drop + CCR retrieval marker).
    let mut big = Vec::new();
    for i in 0..80 {
        big.push(format!(
            r#"{{"id":{i},"name":"account_{i}","email":"a{i}@example.com","tier":"gold"}}"#
        ));
    }
    let big_input = format!("[{}]", big.join(","));
    let out = show(
        "JSON list — 80 rows (row-drop + CCR)",
        "list_accounts",
        &big_input,
    );

    // ── Demonstrate the reversibility: pull the hash from the marker and
    //    retrieve the full original via the CCR store (what the
    //    `retrieve_tool_output` tool does).
    if let Some(marker) = out.lines().find(|l| l.contains("retrieve_tool_output(")) {
        let hash = marker
            .split("retrieve_tool_output(\"")
            .nth(1)
            .and_then(|s| s.split('"').next())
            .unwrap_or("");
        println!("\n─── CCR RETRIEVE round-trip ─────────────────────────────────");
        println!("  marker hash = {hash}");
        match store::retrieve(hash) {
            Some(original) => println!(
                "  retrieve_tool_output(\"{hash}\") → {} bytes restored (matches original: {})",
                original.len(),
                original == big_input
            ),
            None => println!("  (evicted)"),
        }
    }
    println!("\n══════════════════════════════════════════════════════════════\n");
}
