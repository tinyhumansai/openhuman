use super::*;
use chrono::TimeZone;
use serde_json::json;

// ── JobType ────────────────────────────────────────────────────

#[test]
fn job_type_parse_and_as_str_roundtrip() {
    assert_eq!(JobType::parse("shell").as_str(), "shell");
    assert_eq!(JobType::parse("agent").as_str(), "agent");
    assert_eq!(JobType::parse("flow").as_str(), "flow");
    // Case-insensitive
    assert_eq!(JobType::parse("AGENT"), JobType::Agent);
    assert_eq!(JobType::parse("Agent"), JobType::Agent);
    assert_eq!(JobType::parse("FLOW"), JobType::Flow);
    // Anything unknown falls back to Shell (the default) — guards
    // against unexpected legacy DB rows silently turning into Agent.
    assert_eq!(JobType::parse(""), JobType::Shell);
    assert_eq!(JobType::parse("garbage"), JobType::Shell);
}

#[test]
fn job_type_default_is_shell() {
    assert_eq!(JobType::default(), JobType::Shell);
}

#[test]
fn job_type_serializes_lowercase() {
    assert_eq!(serde_json::to_string(&JobType::Shell).unwrap(), "\"shell\"");
    assert_eq!(serde_json::to_string(&JobType::Agent).unwrap(), "\"agent\"");
}

// ── SessionTarget ──────────────────────────────────────────────

#[test]
fn session_target_parse_and_as_str_roundtrip() {
    assert_eq!(SessionTarget::parse("isolated").as_str(), "isolated");
    assert_eq!(SessionTarget::parse("main").as_str(), "main");
    // Case-insensitive + unknown falls back to Isolated (the default).
    assert_eq!(SessionTarget::parse("MAIN"), SessionTarget::Main);
    assert_eq!(SessionTarget::parse(""), SessionTarget::Isolated);
    assert_eq!(SessionTarget::parse("unknown"), SessionTarget::Isolated);
}

#[test]
fn session_target_default_is_isolated() {
    assert_eq!(SessionTarget::default(), SessionTarget::Isolated);
}

#[test]
fn session_target_serializes_lowercase() {
    assert_eq!(
        serde_json::to_string(&SessionTarget::Isolated).unwrap(),
        "\"isolated\""
    );
    assert_eq!(
        serde_json::to_string(&SessionTarget::Main).unwrap(),
        "\"main\""
    );
}

// ── Schedule ───────────────────────────────────────────────────

#[test]
fn schedule_cron_variant_roundtrips_with_optional_tz() {
    let s = Schedule::Cron {
        expr: "0 9 * * *".into(),
        tz: Some("America/Los_Angeles".into()),
        active_hours: None,
    };
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["kind"], "cron");
    assert_eq!(v["expr"], "0 9 * * *");
    assert_eq!(v["tz"], "America/Los_Angeles");
    let back: Schedule = serde_json::from_value(v).unwrap();
    assert_eq!(back, s);
}

#[test]
fn schedule_cron_variant_accepts_missing_tz() {
    let raw = json!({ "kind": "cron", "expr": "*/5 * * * *" });
    let s: Schedule = serde_json::from_value(raw).unwrap();
    assert_eq!(
        s,
        Schedule::Cron {
            expr: "*/5 * * * *".into(),
            tz: None,
            active_hours: None,
        }
    );
}

#[test]
fn schedule_cron_variant_roundtrips_with_active_hours() {
    let s = Schedule::Cron {
        expr: "*/15 * * * *".into(),
        tz: Some("UTC".into()),
        active_hours: Some(ActiveHours {
            start: "09:00".into(),
            end: "17:30".into(),
        }),
    };
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["active_hours"]["start"], "09:00");
    assert_eq!(v["active_hours"]["end"], "17:30");
    let back: Schedule = serde_json::from_value(v).unwrap();
    assert_eq!(back, s);
}

#[test]
fn schedule_at_variant_roundtrips_with_utc_timestamp() {
    let at = Utc.with_ymd_and_hms(2027, 1, 15, 12, 0, 0).unwrap();
    let s = Schedule::At { at };
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["kind"], "at");
    let back: Schedule = serde_json::from_value(v).unwrap();
    assert_eq!(back, s);
}

#[test]
fn schedule_every_variant_roundtrips() {
    let s = Schedule::Every { every_ms: 60_000 };
    let v = serde_json::to_value(&s).unwrap();
    assert_eq!(v["kind"], "every");
    assert_eq!(v["every_ms"], 60_000);
    let back: Schedule = serde_json::from_value(v).unwrap();
    assert_eq!(back, s);
}

// ── Schedule bare-string deserialization (CORE-RUST-FY fix) ──────
// Callers (agents, older frontend) sometimes pass a bare cron
// expression string like `"0 9 * * 1"` instead of the structured
// `{"kind":"cron","expr":"0 9 * * 1"}` form.  Both must parse.

#[test]
fn schedule_deserializes_bare_cron_string() {
    let s: Schedule = serde_json::from_value(json!("0 9 * * 1")).unwrap();
    assert_eq!(
        s,
        Schedule::Cron {
            expr: "0 9 * * 1".into(),
            tz: None,
            active_hours: None,
        }
    );
}

#[test]
fn schedule_deserializes_bare_5_field_cron_string() {
    let s: Schedule = serde_json::from_str("\"*/5 * * * *\"").unwrap();
    assert_eq!(
        s,
        Schedule::Cron {
            expr: "*/5 * * * *".into(),
            tz: None,
            active_hours: None,
        }
    );
}

#[test]
fn cron_job_patch_accepts_bare_schedule_string() {
    // This is the exact payload shape that triggered CORE-RUST-FY:
    // {"schedule": "0 9 * * 1"}
    let raw = json!({ "schedule": "0 9 * * 1" });
    let patch: CronJobPatch = serde_json::from_value(raw).unwrap();
    assert_eq!(
        patch.schedule,
        Some(Schedule::Cron {
            expr: "0 9 * * 1".into(),
            tz: None,
            active_hours: None,
        })
    );
}

#[test]
fn cron_job_patch_still_accepts_structured_schedule_object() {
    let raw = json!({ "schedule": { "kind": "cron", "expr": "0 9 * * 1" } });
    let patch: CronJobPatch = serde_json::from_value(raw).unwrap();
    assert_eq!(
        patch.schedule,
        Some(Schedule::Cron {
            expr: "0 9 * * 1".into(),
            tz: None,
            active_hours: None,
        })
    );
}

// ── DeliveryConfig ─────────────────────────────────────────────

#[test]
fn delivery_config_default_is_none_mode_best_effort() {
    let d = DeliveryConfig::default();
    assert_eq!(d.mode, "none");
    assert!(d.channel.is_none());
    assert!(d.to.is_none());
    assert!(d.best_effort, "default best_effort must be true");
}

#[test]
fn delivery_config_parses_empty_object_with_defaults() {
    // A bare `{}` must deserialize with the `#[serde(default)]` / default
    // fn fallbacks — otherwise legacy rows without delivery fields would
    // fail to load.
    let d: DeliveryConfig = serde_json::from_str("{}").unwrap();
    assert_eq!(d.mode, "");
    assert!(d.channel.is_none());
    assert!(d.to.is_none());
    assert!(d.best_effort, "best_effort must default to true");
}

#[test]
fn delivery_config_preserves_best_effort_false_override() {
    let raw = json!({ "mode": "channel", "best_effort": false });
    let d: DeliveryConfig = serde_json::from_value(raw).unwrap();
    assert_eq!(d.mode, "channel");
    assert!(!d.best_effort);
}

// ── CronJobPatch ───────────────────────────────────────────────

#[test]
fn cron_job_patch_default_is_all_none() {
    let p = CronJobPatch::default();
    assert!(p.schedule.is_none());
    assert!(p.command.is_none());
    assert!(p.prompt.is_none());
    assert!(p.name.is_none());
    assert!(p.enabled.is_none());
    assert!(p.delivery.is_none());
    assert!(p.model.is_none());
    assert!(p.session_target.is_none());
    assert!(p.delete_after_run.is_none());
    assert!(p.agent_id.is_none());
}

#[test]
fn cron_job_deserializes_without_profile_id() {
    // A pre-2b serialized CronJob (no `profile_id` key) must still
    // deserialize, with the field defaulting to None.
    let raw = json!({
        "id": "j1",
        "expression": "0 9 * * *",
        "schedule": { "kind": "cron", "expr": "0 9 * * *" },
        "command": "",
        "prompt": "hi",
        "name": "briefing",
        "job_type": "agent",
        "session_target": "isolated",
        "model": null,
        "agent_id": null,
        "enabled": true,
        "delivery": {},
        "delete_after_run": false,
        "created_at": "2027-01-15T12:00:00Z",
        "next_run": "2027-01-16T09:00:00Z",
        "last_run": null,
        "last_status": null,
        "last_output": null
    });
    let job: CronJob = serde_json::from_value(raw).unwrap();
    assert_eq!(job.profile_id, None);
}

#[test]
fn cron_job_patch_default_leaves_profile_id_none() {
    assert!(CronJobPatch::default().profile_id.is_none());
}

#[test]
fn cron_job_patch_profile_id_supports_explicit_none_clearing() {
    // Option<Option<String>>: None = no change, Some(None) = clear.
    let clear = CronJobPatch {
        profile_id: Some(None),
        ..Default::default()
    };
    assert!(clear.profile_id.is_some());
    assert!(clear.profile_id.as_ref().unwrap().is_none());

    let set = CronJobPatch {
        profile_id: Some(Some("alice".into())),
        ..Default::default()
    };
    assert_eq!(set.profile_id, Some(Some("alice".to_string())));
}

#[test]
fn patch_profile_id_wire_double_option_semantics() {
    // The RPC path deserializes CronJobPatch from JSON params — pin the three
    // wire cases so a `null` clears rather than silently no-ops.
    // absent key → no change.
    let absent: CronJobPatch = serde_json::from_value(json!({})).unwrap();
    assert_eq!(absent.profile_id, None, "absent key means no change");
    // present null → clear.
    let cleared: CronJobPatch = serde_json::from_value(json!({ "profile_id": null })).unwrap();
    assert_eq!(
        cleared.profile_id,
        Some(None),
        "wire null must clear the attribution"
    );
    // present value → set.
    let set: CronJobPatch = serde_json::from_value(json!({ "profile_id": "writer" })).unwrap();
    assert_eq!(set.profile_id, Some(Some("writer".to_string())));
}

#[test]
fn patch_agent_id_wire_double_option_semantics() {
    // Same fix applied consistently to `agent_id` (its doc + the struct-level
    // clearing test already document the Some(None)=clear intent).
    let absent: CronJobPatch = serde_json::from_value(json!({})).unwrap();
    assert_eq!(absent.agent_id, None, "absent key means no change");
    let cleared: CronJobPatch = serde_json::from_value(json!({ "agent_id": null })).unwrap();
    assert_eq!(
        cleared.agent_id,
        Some(None),
        "wire null must clear the agent definition"
    );
    let set: CronJobPatch = serde_json::from_value(json!({ "agent_id": "welcome" })).unwrap();
    assert_eq!(set.agent_id, Some(Some("welcome".to_string())));
}

#[test]
fn cron_job_patch_agent_id_supports_explicit_none_clearing() {
    // Option<Option<String>> lets callers distinguish "no change"
    // (None) from "clear the agent_id" (Some(None)).
    let p = CronJobPatch {
        agent_id: Some(None),
        ..Default::default()
    };
    assert!(p.agent_id.is_some());
    assert!(p.agent_id.as_ref().unwrap().is_none());
}
