//! RPC-level e2e coverage for `openhuman.team_*`, `openhuman.referral_*`,
//! `openhuman.update_*` and `openhuman.migrate_openclaw`.
//!
//! Team and referral are pure proxies over the hosted API, so they are driven
//! against an in-process mock backend and asserted on the *shape of the request
//! the adapter built* as well as the response it returned. Update and migrate
//! are local, and are asserted without any network at all.
//!
//! A module of the aggregated `raw_coverage_all` target, not a target of its
//! own. Run with:
//!   cargo test --test raw_coverage_all --features "$(bash scripts/ci/product-features.sh)" \
//!       -- team_referral_e2e

#[path = "w4_shared/mod.rs"]
mod support;

use serde_json::{json, Value};
use support::{assert_no_error, error_message, logs, mock_log, peel, Harness};

// ── team ─────────────────────────────────────────────────────────────────────

/// The nine team controllers that had no e2e target, end to end against the
/// mock backend, plus the request each one actually put on the wire.
#[tokio::test]
async fn team_uncovered_controllers_round_trip_against_the_backend() {
    let _lock = support::env_lock();
    let harness = Harness::start("", false).await;
    let log = mock_log();
    log.clear();
    harness.login().await;

    // --- team_get_usage -----------------------------------------------------
    let usage = harness.call(10, "openhuman.team_get_usage", json!({})).await;
    let usage = peel(assert_no_error(&usage, "team_get_usage"));
    assert_eq!(
        usage.get("seatsUsed").and_then(Value::as_u64),
        Some(2),
        "usage must surface the backend payload, not a placeholder: {usage}"
    );
    assert_eq!(usage.get("spendUsd").and_then(Value::as_f64), Some(12.25));

    // --- team_list_teams ----------------------------------------------------
    let teams = harness.call(11, "openhuman.team_list_teams", json!({})).await;
    let teams = peel(assert_no_error(&teams, "team_list_teams"));
    let rows = teams
        .as_array()
        .unwrap_or_else(|| panic!("team_list_teams must return an array: {teams}"));
    assert_eq!(rows.len(), 2, "seeded two teams: {teams}");
    assert_eq!(rows[0].get("name").and_then(Value::as_str), Some("Alpha"));
    assert_eq!(rows[1].get("role").and_then(Value::as_str), Some("MEMBER"));

    // --- team_get_team ------------------------------------------------------
    let team = harness
        .call(12, "openhuman.team_get_team", json!({ "teamId": "team-1" }))
        .await;
    let team = peel(assert_no_error(&team, "team_get_team"));
    assert_eq!(team.get("id").and_then(Value::as_str), Some("team-1"));
    assert_eq!(
        team.get("memberCount").and_then(Value::as_u64),
        Some(2),
        "the whole backend row must survive the proxy: {team}"
    );

    // --- team_create_team ---------------------------------------------------
    let created = harness
        .call(
            13,
            "openhuman.team_create_team",
            json!({ "name": "  Gamma  " }),
        )
        .await;
    let created_outer = assert_no_error(&created, "team_create_team");
    assert!(
        logs(created_outer)
            .iter()
            .any(|l| l == "team created via backend"),
        "the outcome must carry its log line: {created_outer}"
    );
    let created = peel(created_outer);
    assert_eq!(
        created.get("name").and_then(Value::as_str),
        Some("Gamma"),
        "the adapter trims the name before sending; the mock echoes what it got: {created}"
    );

    // --- team_update_team: a blank name is dropped, not sent as "" ----------
    let renamed = harness
        .call(
            14,
            "openhuman.team_update_team",
            json!({ "teamId": "team-1", "name": "Renamed" }),
        )
        .await;
    let renamed = peel(assert_no_error(&renamed, "team_update_team"));
    assert_eq!(
        renamed.get("name").and_then(Value::as_str),
        Some("Renamed"),
        "the new name must reach the backend body: {renamed}"
    );

    let blank_rename = harness
        .call(
            15,
            "openhuman.team_update_team",
            json!({ "teamId": "team-1", "name": "   " }),
        )
        .await;
    let blank_rename = peel(assert_no_error(&blank_rename, "team_update_team blank name"));
    assert!(
        blank_rename.get("name").is_some_and(Value::is_null),
        "a whitespace-only name is documented as dropped from the PUT body, not \
         sent as an empty string that would blank the team's name: {blank_rename}"
    );

    // --- team_switch_team ---------------------------------------------------
    let switched = harness
        .call(
            16,
            "openhuman.team_switch_team",
            json!({ "teamId": "team-2" }),
        )
        .await;
    let switched = peel(assert_no_error(&switched, "team_switch_team"));
    assert_eq!(
        switched.get("activeTeamId").and_then(Value::as_str),
        Some("team-2"),
        "switching must name the team that became active: {switched}"
    );

    // --- team_leave_team ----------------------------------------------------
    let left = harness
        .call(17, "openhuman.team_leave_team", json!({ "teamId": "team-2" }))
        .await;
    let left = peel(assert_no_error(&left, "team_leave_team"));
    assert_eq!(left.get("left").and_then(Value::as_str), Some("team-2"));

    // --- team_join_team -----------------------------------------------------
    let joined = harness
        .call(18, "openhuman.team_join_team", json!({ "code": "JOIN-OK" }))
        .await;
    let joined = peel(assert_no_error(&joined, "team_join_team"));
    assert_eq!(joined.get("joined").and_then(Value::as_bool), Some(true));
    assert_eq!(
        joined.get("role").and_then(Value::as_str),
        Some("MEMBER"),
        "the role the backend assigned must reach the caller: {joined}"
    );

    // --- team_delete_team ---------------------------------------------------
    let deleted = harness
        .call(
            19,
            "openhuman.team_delete_team",
            json!({ "teamId": "team-1" }),
        )
        .await;
    let deleted = peel(assert_no_error(&deleted, "team_delete_team"));
    assert_eq!(deleted.get("deleted").and_then(Value::as_str), Some("team-1"));

    // Each verb reached its documented endpoint with its documented method.
    let calls: Vec<(String, String)> = log
        .entries()
        .iter()
        .map(|e| {
            (
                e.get("method")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string(),
                e.get("path").and_then(Value::as_str).unwrap_or("").to_string(),
            )
        })
        .collect();
    for expected in [
        ("GET", "/teams/me/usage"),
        ("GET", "/teams"),
        ("GET", "/teams/team-1"),
        ("POST", "/teams"),
        ("PUT", "/teams/team-1"),
        ("POST", "/teams/team-2/switch"),
        ("POST", "/teams/team-2/leave"),
        ("POST", "/teams/join"),
        ("DELETE", "/teams/team-1"),
    ] {
        assert!(
            calls.contains(&(expected.0.to_string(), expected.1.to_string())),
            "expected a {} {} on the wire; saw {calls:?}",
            expected.0,
            expected.1
        );
    }
}

/// Backend rejections must surface, and the id validation in `team/ops.rs` must
/// fire before any request — including the path-injection guard.
#[tokio::test]
async fn team_rejects_bad_ids_and_surfaces_backend_failures() {
    let _lock = support::env_lock();
    let harness = Harness::start("", false).await;
    harness.login().await;
    let log = mock_log();

    // A team the backend does not have.
    let missing = harness
        .call(
            20,
            "openhuman.team_get_team",
            json!({ "teamId": "missing-team" }),
        )
        .await;
    let message = error_message(&missing, "team_get_team 404");
    assert!(
        message.contains("404") && message.contains("team not found"),
        "a backend 404 must surface with its status and body, not as a generic \
         'backend request failed'; got: {message}"
    );

    // An unknown invite code.
    let bad_code = harness
        .call(21, "openhuman.team_join_team", json!({ "code": "WRONG" }))
        .await;
    assert!(
        error_message(&bad_code, "team_join_team bad code").contains("invite code not found"),
        "the backend's reason must reach the user: {bad_code}"
    );

    // An id that would escape its path segment if it were interpolated raw.
    // `build_api_path` pushes it through `path_segments_mut`, which percent-encodes
    // the separators, so the request must arrive as ONE segment.
    log.clear();
    let injected = harness
        .call(
            22,
            "openhuman.team_get_team",
            json!({ "teamId": "a/../admin" }),
        )
        .await;
    let injected = peel(assert_no_error(&injected, "team_get_team with a traversal id"));
    assert_eq!(
        injected.get("id").and_then(Value::as_str),
        Some("a/../admin"),
        "the id must survive encoding and decode back intact: {injected}"
    );
    let raw_path = log
        .first_with_path_prefix("/teams/")
        .and_then(|e| e.get("path").and_then(Value::as_str).map(str::to_string))
        .expect("the lookup must have reached the backend");
    assert_eq!(
        raw_path, "/teams/a%2F..%2Fadmin",
        "the separators must be percent-encoded into a single path segment — an \
         unencoded `/teams/a/../admin` is what a server would resolve to \
         `/teams/admin`, i.e. a different team: {raw_path}"
    );
    assert_eq!(
        raw_path.split('/').count(),
        3,
        "`/` + `teams` + one id segment; anything more means the id escaped: {raw_path}"
    );

    // Local validation, before any network call.
    log.clear();
    for (id, method, params) in [
        (
            30,
            "openhuman.team_get_team",
            json!({ "teamId": "   " }),
        ),
        (
            31,
            "openhuman.team_delete_team",
            json!({ "teamId": "" }),
        ),
        (
            32,
            "openhuman.team_switch_team",
            json!({ "teamId": "  " }),
        ),
    ] {
        let response = harness.call(id, method, params).await;
        assert!(
            error_message(&response, method).contains("teamId is required"),
            "{method} must reject a blank teamId locally: {response}"
        );
    }
    let blank_name = harness
        .call(33, "openhuman.team_create_team", json!({ "name": " " }))
        .await;
    assert!(
        error_message(&blank_name, "team_create_team blank").contains("name is required"),
        "a blank team name must be rejected locally: {blank_name}"
    );
    let blank_join = harness
        .call(34, "openhuman.team_join_team", json!({ "code": "" }))
        .await;
    assert!(
        error_message(&blank_join, "team_join_team blank").contains("code is required"),
        "a blank invite code must be rejected locally: {blank_join}"
    );

    assert!(
        log.entries().is_empty(),
        "validation is documented as pre-HTTP, yet these reached the backend: {:?}",
        log.entries()
    );
}

// ── referral ─────────────────────────────────────────────────────────────────

/// Both referral controllers, happy path and the two failure shapes.
#[tokio::test]
async fn referral_stats_and_claim_round_trip() {
    let _lock = support::env_lock();
    let harness = Harness::start("", false).await;
    let log = mock_log();
    log.clear();
    harness.login().await;

    let stats = harness
        .call(40, "openhuman.referral_get_stats", json!({}))
        .await;
    let stats_outer = assert_no_error(&stats, "referral_get_stats");
    assert!(
        logs(stats_outer)
            .iter()
            .any(|l| l.contains("GET /referral/stats")),
        "the log line names the endpoint it called: {stats_outer}"
    );
    let stats = peel(stats_outer);
    assert_eq!(stats.get("code").and_then(Value::as_str), Some("W4REF"));
    assert_eq!(
        stats.get("referredCount").and_then(Value::as_u64),
        Some(3),
        "referral counts must survive the proxy: {stats}"
    );

    // The optional device fingerprint is only sent when non-blank.
    let claimed = harness
        .call(
            41,
            "openhuman.referral_claim",
            json!({ "code": "  W4REF  ", "deviceFingerprint": "fp-123" }),
        )
        .await;
    let claimed = peel(assert_no_error(&claimed, "referral_claim"));
    assert_eq!(
        claimed.get("code").and_then(Value::as_str),
        Some("W4REF"),
        "the code is trimmed before it is sent: {claimed}"
    );
    assert_eq!(claimed.get("creditsUsd").and_then(Value::as_f64), Some(5.0));

    let with_fp = log
        .first_with_path_prefix("/referral/claim")
        .expect("no claim reached the backend");
    assert_eq!(
        with_fp
            .get("body")
            .and_then(|b| b.get("deviceFingerprint"))
            .and_then(Value::as_str),
        Some("fp-123"),
        "a supplied fingerprint must be forwarded: {with_fp}"
    );

    log.clear();
    let no_fp = harness
        .call(
            42,
            "openhuman.referral_claim",
            json!({ "code": "W4REF", "deviceFingerprint": "   " }),
        )
        .await;
    assert_no_error(&no_fp, "referral_claim blank fingerprint");
    let body = log
        .first_with_path_prefix("/referral/claim")
        .expect("no claim reached the backend");
    assert!(
        body.get("body")
            .and_then(|b| b.get("deviceFingerprint"))
            .is_none(),
        "a whitespace-only fingerprint is documented as omitted from the body \
         rather than sent as an empty string: {body}"
    );

    // No local validation on the code — the backend's rejection is what the
    // caller sees. Asserted so a later change of mind here is visible.
    let blank = harness
        .call(43, "openhuman.referral_claim", json!({ "code": "  " }))
        .await;
    assert!(
        error_message(&blank, "referral_claim blank code").contains("referral code is required"),
        "a blank referral code currently reaches the backend and is rejected \
         there; the message the user sees must still be actionable: {blank}"
    );
}

/// Referral, like billing, must refuse locally when no session is stored.
#[tokio::test]
async fn referral_without_a_session_refuses_locally() {
    let _lock = support::env_lock();
    let harness = Harness::start("", false).await;
    let log = mock_log();
    log.clear();

    let stats = harness
        .call(50, "openhuman.referral_get_stats", json!({}))
        .await;
    let message = error_message(&stats, "referral_get_stats with no session");
    assert!(
        message.contains("auth_store_session"),
        "the error must tell the caller how to recover; got: {message}"
    );
    assert!(
        !log.entries().iter().any(|e| e
            .get("path")
            .and_then(Value::as_str)
            .is_some_and(|p| p.starts_with("/referral"))),
        "a session-less referral call must not reach /referral: {:?}",
        log.entries()
    );
}

// ── update ───────────────────────────────────────────────────────────────────

/// `update_version` is the cheap, no-network probe the frontend gates the other
/// two on. Its three fields have to agree with each other and with the crate.
#[tokio::test]
async fn update_version_reports_the_running_binary() {
    let _lock = support::env_lock();
    let harness = Harness::start("", true).await;

    let version = harness.call(60, "openhuman.update_version", json!({})).await;
    let version = peel(assert_no_error(&version, "update_version"));

    let reported = version
        .get("version")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("update_version must report a version: {version}"));
    assert!(
        reported.split('.').count() >= 3 && reported.chars().next().is_some_and(|c| c.is_ascii_digit()),
        "the version must be the crate's semver, not a placeholder; got {reported:?}"
    );

    let triple = version
        .get("target_triple")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("update_version must report a target triple: {version}"));
    assert!(
        !triple.is_empty() && triple.contains('-'),
        "the target triple must look like a triple; got {triple:?}"
    );

    assert_eq!(
        version.get("asset_prefix").and_then(Value::as_str),
        Some(format!("openhuman-core-{triple}").as_str()),
        "the asset prefix is what the updater matches release assets on, so it \
         must be derived from the same triple it just reported: {version}"
    );
}

/// `update_run` is gated by `config.update.rpc_mutations_enabled`. With the gate
/// closed it must refuse *before* reaching the network, and say why.
#[tokio::test]
async fn update_run_refuses_when_rpc_mutations_are_disabled() {
    let _lock = support::env_lock();
    let harness = Harness::start(
        r#"
[update]
rpc_mutations_enabled = false
"#,
        true,
    )
    .await;

    let run = harness.call(61, "openhuman.update_run", json!({})).await;
    // The policy refusal is reported inside the payload, not as an RPC error,
    // so the frontend can render `applied`/`restart_requested` uniformly.
    let run = peel(assert_no_error(&run, "update_run"));
    let error = run
        .get("error")
        .and_then(Value::as_str)
        .unwrap_or_else(|| panic!("a blocked update_run must report an `error`: {run}"));
    assert!(
        error.contains("rpc_mutations_enabled=false"),
        "the refusal must name the setting that caused it: {error}"
    );
    assert!(
        error.contains("update.check"),
        "the refusal must point at the discovery path that is still allowed: {error}"
    );
    assert_eq!(
        run.get("applied").and_then(Value::as_bool),
        Some(false),
        "a blocked run must not claim it applied anything: {run}"
    );
    assert_eq!(
        run.get("restart_requested").and_then(Value::as_bool),
        Some(false),
        "a blocked run must not ask the supervisor to restart: {run}"
    );
}

/// `update_check` never fails the RPC — it folds a transport failure into an
/// `{error}` payload — so this asserts it lands in exactly one of the two
/// documented shapes, and that the success shape agrees with `update_version`.
///
/// This is the weakest case in the file, and deliberately so: `check_available`
/// hard-codes `https://api.github.com/...` with no override, so an offline or
/// rate-limited run can only take the error branch. See
/// `~/tinyhuman/bugs/e2e-wave-update-check-unmockable.md`.
#[tokio::test]
async fn update_check_lands_in_one_of_its_two_documented_shapes() {
    let _lock = support::env_lock();
    let harness = Harness::start("", true).await;

    let version = harness.call(70, "openhuman.update_version", json!({})).await;
    let running = peel(assert_no_error(&version, "update_version"))
        .get("version")
        .and_then(Value::as_str)
        .expect("update_version must report a version")
        .to_string();

    let check = harness.call(71, "openhuman.update_check", json!({})).await;
    let check = peel(assert_no_error(&check, "update_check"));
    assert!(
        check.is_object(),
        "update_check must always answer with an object, never a bare string or \
         null, because the frontend indexes into it: {check}"
    );

    match check.get("error").and_then(Value::as_str) {
        Some(error) => {
            assert!(
                !error.is_empty(),
                "the failure shape must carry a non-empty reason: {check}"
            );
            assert!(
                check.get("update_available").is_none(),
                "the failure shape must not also claim an update verdict: {check}"
            );
        }
        None => {
            assert_eq!(
                check.get("current_version").and_then(Value::as_str),
                Some(running.as_str()),
                "the success shape must report the same running version \
                 `update_version` does: {check}"
            );
            assert!(
                check.get("update_available").is_some_and(Value::is_boolean),
                "the success shape must carry a boolean verdict: {check}"
            );
        }
    }
}

// ── migrate ──────────────────────────────────────────────────────────────────

/// `migrate_openclaw` dry-run over a seeded OpenClaw workspace, plus the two
/// refusals it owes the caller.
#[tokio::test]
async fn migrate_openclaw_dry_run_counts_sources_without_importing() {
    let _lock = support::env_lock();
    let harness = Harness::start("", true).await;

    // A source workspace shaped like OpenClaw's: a top-level MEMORY.md and two
    // markdown notes under memory/. An empty file must not be counted.
    let source = harness.home.join("openclaw-workspace");
    std::fs::create_dir_all(source.join("memory")).expect("create the source workspace");
    std::fs::write(source.join("MEMORY.md"), "# Top level\nremember this")
        .expect("write MEMORY.md");
    std::fs::write(source.join("memory").join("alpha.md"), "alpha note")
        .expect("write alpha.md");
    std::fs::write(source.join("memory").join("beta.md"), "beta note").expect("write beta.md");
    std::fs::write(source.join("memory").join("blank.md"), "   \n").expect("write blank.md");
    std::fs::write(source.join("memory").join("ignored.txt"), "not markdown")
        .expect("write ignored.txt");

    let report = harness
        .call(
            80,
            "openhuman.migrate_openclaw",
            json!({ "source_workspace": source.to_string_lossy(), "dry_run": true }),
        )
        .await;
    let report_outer = assert_no_error(&report, "migrate_openclaw dry run");
    assert!(
        logs(report_outer).iter().any(|l| l == "migration completed"),
        "the outcome must carry its log line: {report_outer}"
    );
    let report = peel(report_outer);

    assert_eq!(
        report.get("dry_run").and_then(Value::as_bool),
        Some(true),
        "a dry run must say so in its report: {report}"
    );
    let stats = report
        .get("stats")
        .unwrap_or_else(|| panic!("the report must carry `stats`: {report}"));
    assert_eq!(
        stats.get("from_markdown").and_then(Value::as_u64),
        Some(3),
        "MEMORY.md plus two non-empty notes; the blank note and the .txt must \
         not be counted: {stats}"
    );
    assert_eq!(
        stats.get("from_sqlite").and_then(Value::as_u64),
        Some(0),
        "there is no brain.db in this fixture: {stats}"
    );
    assert_eq!(
        stats.get("imported").and_then(Value::as_u64),
        Some(0),
        "a dry run must import nothing: {stats}"
    );
    assert_eq!(
        report.get("source_workspace").and_then(Value::as_str),
        Some(source.to_string_lossy().as_ref()),
        "the report must name the source it read: {report}"
    );

    // The dry run promised to touch nothing; check the source rather than
    // taking the flag on trust.
    assert_eq!(
        std::fs::read_to_string(source.join("MEMORY.md")).expect("read back MEMORY.md"),
        "# Top level\nremember this",
        "a dry run must leave the source workspace byte-identical"
    );

    // An absent source is an error, not an empty success.
    let missing = harness
        .call(
            81,
            "openhuman.migrate_openclaw",
            json!({
                "source_workspace": harness.home.join("no-such-workspace").to_string_lossy(),
                "dry_run": true
            }),
        )
        .await;
    let message = error_message(&missing, "migrate_openclaw missing source");
    assert!(
        message.contains("OpenClaw workspace not found"),
        "an absent source must be named, so the user can correct the path; got: {message}"
    );

    // Migrating a workspace into itself would duplicate every entry.
    let workspace = harness.workspace();
    let self_migrate = harness
        .call(
            82,
            "openhuman.migrate_openclaw",
            json!({ "source_workspace": workspace.to_string_lossy(), "dry_run": true }),
        )
        .await;
    assert!(
        error_message(&self_migrate, "migrate_openclaw self").contains("refusing self-migration"),
        "pointing the migration at the active workspace must be refused: {self_migrate}"
    );
}

/// A source workspace with nothing importable reports *why* it found nothing.
#[tokio::test]
async fn migrate_openclaw_reports_where_it_looked_when_it_finds_nothing() {
    let _lock = support::env_lock();
    let harness = Harness::start("", true).await;

    let source = harness.home.join("empty-openclaw");
    std::fs::create_dir_all(&source).expect("create the empty source workspace");

    let report = harness
        .call(
            90,
            "openhuman.migrate_openclaw",
            json!({ "source_workspace": source.to_string_lossy(), "dry_run": true }),
        )
        .await;
    let report = peel(assert_no_error(&report, "migrate_openclaw empty source"));

    let warnings: Vec<&str> = report
        .get("warnings")
        .and_then(Value::as_array)
        .unwrap_or_else(|| panic!("the report must carry `warnings`: {report}"))
        .iter()
        .filter_map(Value::as_str)
        .collect();
    assert!(
        warnings.iter().any(|w| w.contains("No importable memory")),
        "an empty source must warn rather than silently report success: {warnings:?}"
    );
    assert!(
        warnings
            .iter()
            .any(|w| w.contains("memory/brain.db") && w.contains("MEMORY.md")),
        "the warning must list the paths it checked, so a user with data \
         elsewhere knows why it was missed: {warnings:?}"
    );
    assert_eq!(
        report
            .get("stats")
            .and_then(|s| s.get("imported"))
            .and_then(Value::as_u64),
        Some(0)
    );
}
