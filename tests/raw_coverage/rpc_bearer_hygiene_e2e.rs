//! #6112 regression guard: no aggregated suite may send a hard-coded bearer.
//!
//! `core::auth::RPC_TOKEN` is a process-global `OnceLock` and `init_rpc_token`
//! returns early once it is set — deliberately, so a second call cannot 401 live
//! clients. Since `tests/raw_coverage/` is one binary, only the first suite to
//! initialise it pins its own `TEST_RPC_TOKEN`; any suite that then sends its own
//! literal is answered `401` and trips its own `assert_eq!(status, OK)`. Which
//! suites fail depends on libtest scheduling, so the failure is load-dependent
//! and invisible to CI, which runs one process per module filter.
//!
//! The fix is for every suite to send `get_rpc_token()` — the token the process
//! actually validates — rather than the literal it hoped to install. This scans
//! the sibling sources so a *new* suite cannot reintroduce the literal, which a
//! runtime assertion in one file could not do: that assertion passes whenever its
//! own file happens to win the scheduling race.

use std::path::Path;

/// Does this line send the suite's own `TEST_RPC_TOKEN` as a bearer?
///
/// Matches on the *combination* rather than on fixed spellings. An earlier
/// version listed two literal forms and missed `format!("Bearer {}",
/// TEST_RPC_TOKEN)` and `bearer_auth(&TEST_RPC_TOKEN)`, which send the same
/// wrong token (thanks to CodeRabbit on #6124 for catching it). Any line that
/// names the suite-local constant *and* an authorization-sending construct is
/// the bug, however it is spelled — including spellings nobody has invented yet.
///
/// Comments are exempt: several suites legitimately explain the hazard in prose.
fn line_sends_local_token(line: &str) -> bool {
    let code = line.trim_start();
    if code.starts_with("//") {
        return false;
    }
    code.contains("TEST_RPC_TOKEN") && (code.contains("Bearer") || code.contains("bearer_auth"))
}

#[test]
fn the_detector_catches_every_spelling_of_the_bug() {
    // The two forms the original guard caught.
    assert!(line_sends_local_token(
        r#"        .bearer_auth(TEST_RPC_TOKEN)"#
    ));
    assert!(line_sends_local_token(
        r#"        .header(AUTHORIZATION, format!("Bearer {TEST_RPC_TOKEN}"))"#
    ));
    // The two it missed.
    assert!(line_sends_local_token(
        r#"        .header(AUTHORIZATION, format!("Bearer {}", TEST_RPC_TOKEN))"#
    ));
    assert!(line_sends_local_token(
        r#"        .bearer_auth(&TEST_RPC_TOKEN)"#
    ));
    // And a spelling none of the suites use today.
    assert!(line_sends_local_token(
        r#"        let h = String::from("Bearer ") + TEST_RPC_TOKEN;"#
    ));

    // Must NOT fire: the correct call, the seed, the declaration, and prose.
    assert!(!line_sends_local_token(
        r#"        .header(AUTHORIZATION, format!("Bearer {}", rpc_bearer()))"#
    ));
    assert!(!line_sends_local_token(
        r#"        std::env::set_var(CORE_TOKEN_ENV_VAR, TEST_RPC_TOKEN);"#
    ));
    assert!(!line_sends_local_token(
        r#"const TEST_RPC_TOKEN: &str = "connectivity-raw-coverage-e2e-token";"#
    ));
    assert!(!line_sends_local_token(
        r#"/// suite sending its own TEST_RPC_TOKEN as a Bearer gets a 401."#
    ));
}

#[test]
fn no_raw_coverage_suite_sends_a_hard_coded_bearer() {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/raw_coverage");
    let mut offenders = Vec::new();
    let mut scanned = 0usize;

    for entry in std::fs::read_dir(&dir).expect("tests/raw_coverage must be readable") {
        let path = entry.expect("dir entry").path();
        if path.extension().and_then(|e| e.to_str()) != Some("rs") {
            continue;
        }
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or_default()
            .to_string();
        if name == "rpc_bearer_hygiene_e2e.rs" {
            continue; // this file names the forbidden forms on purpose
        }
        let source = std::fs::read_to_string(&path).expect("suite source must be readable");
        scanned += 1;
        for (i, line) in source.lines().enumerate() {
            if line_sends_local_token(line) {
                offenders.push(format!("{name}:{}: {}", i + 1, line.trim()));
            }
        }
    }

    // An empty scan is a failure, not a pass: if the glob ever stops matching,
    // this test would otherwise report clean having read nothing.
    assert!(
        scanned > 10,
        "expected to scan the raw_coverage suites, only saw {scanned} file(s) in {}",
        dir.display()
    );
    assert!(
        offenders.is_empty(),
        "raw_coverage suites must send `core::auth::get_rpc_token()`, not their own \
         `TEST_RPC_TOKEN` literal — only the first suite in the process wins that race \
         (#6112):\n  {}",
        offenders.join("\n  ")
    );
}
