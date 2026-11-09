use serde_json::{json, Value};

use super::args::{parse_args, DoctorArgs};
use super::is_doctor_command;
use super::runner::exit_code;

fn args(values: &[&str]) -> DoctorArgs {
    let values = values
        .iter()
        .map(|value| (*value).to_owned())
        .collect::<Vec<_>>();
    parse_args(&values).expect("valid doctor arguments")
}

fn report(findings: Value) -> Value {
    json!({
        "schema_version": 2,
        "profile": null,
        "config_source": "profile",
        "tier": "connect",
        "checks_run": [],
        "checks_skipped": [],
        "findings": findings,
        "summary": { "pass": 0, "info": 0, "warn": 0, "error": 0, "skipped": 0 },
        "ok": true
    })
}

#[test]
fn defaults_to_connect_tier_and_error_lint_threshold() {
    let options = args(&[]);

    assert_eq!(options.tier, "connect");
    assert_eq!(options.severity_min, "error");
    assert!(!options.json);
    assert!(!options.lint);
    assert!(options.only.is_empty());
    assert!(options.skip.is_empty());
    assert!(options.profile.is_none());
}

#[test]
fn invalid_severity_and_unknown_flags_are_parse_errors() {
    let invalid_severity = ["--severity-min", "pass"].map(str::to_owned);
    let unknown_flag = ["--force"].map(str::to_owned);

    assert!(parse_args(&invalid_severity).is_err());
    assert!(parse_args(&unknown_flag).is_err());
}

#[test]
fn lint_defaults_to_error_threshold() {
    let options = args(&["--lint"]);
    let warnings = report(json!([{
        "check_id": "config/provenance",
        "severity": "warn",
        "message": "The setting came from an implicit source.",
        "fix_hint": "Set the intended source explicitly.",
        "duration_ms": 1,
        "affects": []
    }]));

    assert_eq!(exit_code(&options, Ok(&warnings)), 0);
}

#[test]
fn lint_error_exits_one() {
    let options = args(&["--lint"]);
    let errors = report(json!([{
        "check_id": "config/file-integrity",
        "severity": "error",
        "message": "The config file is invalid.",
        "fix_hint": "Repair the config file.",
        "duration_ms": 1,
        "affects": []
    }]));

    assert_eq!(exit_code(&options, Ok(&errors)), 1);
}

#[test]
fn lint_warning_exits_one_when_warn_is_the_minimum() {
    let options = args(&["--lint", "--severity-min", "warn"]);
    let warnings = report(json!([{
        "check_id": "config/provenance",
        "severity": "warn",
        "message": "The setting came from an implicit source.",
        "fix_hint": "Set the intended source explicitly.",
        "duration_ms": 1,
        "affects": []
    }]));

    assert_eq!(exit_code(&options, Ok(&warnings)), 1);
}

#[test]
fn lint_json_still_uses_the_requested_severity_threshold() {
    let options = args(&["--lint", "--json", "--severity-min", "warn"]);
    let warnings = report(json!([{
        "check_id": "config/provenance",
        "severity": "warn",
        "message": "The setting came from an implicit source.",
        "fix_hint": "Set the intended source explicitly.",
        "duration_ms": 1,
        "affects": []
    }]));

    assert_eq!(exit_code(&options, Ok(&warnings)), 1);
}

#[test]
fn runtime_failure_exits_two_before_a_report_exists() {
    let options = args(&["--lint"]);

    assert_eq!(exit_code(&options, Err(())), 2);
}

#[test]
fn unknown_only_selector_error_finding_fails_lint() {
    let options = args(&["--lint", "--only", "unknown/check"]);
    let selection_error = report(json!([{
        "check_id": "doctor/selection",
        "severity": "error",
        "message": "A requested check id is not registered.",
        "fix_hint": "Use a registered exact check id.",
        "duration_ms": 0,
        "affects": []
    }]));

    assert_eq!(exit_code(&options, Ok(&selection_error)), 1);
}

#[test]
fn json_without_lint_is_advisory_even_for_errors() {
    let options = args(&["--json"]);
    let errors = report(json!([{
        "check_id": "config/file-integrity",
        "severity": "error",
        "message": "The config file is invalid.",
        "fix_hint": "Repair the config file.",
        "duration_ms": 1,
        "affects": []
    }]));

    assert_eq!(exit_code(&options, Ok(&errors)), 0);
}

#[test]
fn live_profile_and_repeatable_selectors_map_to_rpc_options() {
    let options = args(&[
        "--live",
        "--profile",
        "desktop-default",
        "--only",
        "config/provenance",
        "--only",
        "workspace/layout",
        "--skip",
        "config/provenance",
    ]);

    assert_eq!(options.tier, "live");
    assert_eq!(options.profile.as_deref(), Some("desktop-default"));
    assert_eq!(options.only, ["config/provenance", "workspace/layout"]);
    assert_eq!(options.skip, ["config/provenance"]);
    assert_eq!(
        options.rpc_params(),
        json!({
            "tier": "live",
            "profile": "desktop-default",
            "only": ["config/provenance", "workspace/layout"],
            "skip": ["config/provenance"]
        })
    );
}

#[test]
fn legacy_doctor_namespace_commands_stay_with_the_core_dispatcher() {
    for command in ["report", "models"] {
        let invocation = vec!["doctor".to_owned(), command.to_owned()];
        assert!(!is_doctor_command(&invocation));
    }
}

#[test]
fn help_flags_ask_for_usage_instead_of_failing() {
    for flag in ["--help", "-h", "help"] {
        assert!(args(&[flag]).help, "{flag}");
        assert!(is_doctor_command(&["doctor".to_owned(), flag.to_owned()]));
    }
    assert!(!args(&[]).help);
    assert!(super::args::USAGE.contains("--severity-min"));
}

#[test]
fn a_failed_run_names_a_safe_reason_class() {
    use super::runner::{failure_output, DoctorFailure};
    for failure in [
        DoctorFailure::Runtime,
        DoctorFailure::Host,
        DoctorFailure::Rpc,
        DoctorFailure::NoReport,
    ] {
        let text = failure_output(false, failure);
        assert!(text.starts_with("doctor could not produce a report: "));
        assert!(text.ends_with(failure.reason()));
        let json: Value = serde_json::from_str(&failure_output(true, failure)).unwrap();
        assert_eq!(json, json!({ "ok": false, "reason": failure.reason() }));
    }
}
