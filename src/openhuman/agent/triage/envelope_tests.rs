use super::*;
use serde_json::json;

#[test]
fn composio_envelope_builds_expected_label_and_slug() {
    let env = TriggerEnvelope::from_composio(
        "gmail",
        "GMAIL_NEW_GMAIL_MESSAGE",
        "trig-1",
        "uuid-1",
        json!({ "from": "a@b.com" }),
    );
    assert_eq!(env.display_label, "composio/gmail/GMAIL_NEW_GMAIL_MESSAGE");
    assert_eq!(env.external_id, "uuid-1");
    assert_eq!(env.source.slug(), "composio");
    match env.source {
        TriggerSource::Composio { toolkit, trigger } => {
            assert_eq!(toolkit, "gmail");
            assert_eq!(trigger, "GMAIL_NEW_GMAIL_MESSAGE");
        }
        _ => panic!("expected Composio variant"),
    }
    assert_eq!(env.payload["from"], "a@b.com");
}

#[test]
fn with_task_card_attaches_card_link() {
    let env =
        TriggerEnvelope::from_external("task_sources:ts-1", "external task ingested", json!({}));
    assert!(env.card_link.is_none(), "no link by default");

    let location = BoardLocation::Thread {
        workspace_dir: std::path::PathBuf::from("/tmp/ws"),
        thread_id: "task-sources".to_string(),
    };
    let linked = env.with_task_card("card-1".to_string(), location);
    let link = linked.card_link.expect("card_link attached");
    assert_eq!(link.card_id, "card-1");
    match link.location {
        BoardLocation::Thread { thread_id, .. } => assert_eq!(thread_id, "task-sources"),
        _ => panic!("expected Thread board location"),
    }
}

#[test]
fn composio_envelope_falls_back_to_metadata_id_when_uuid_missing() {
    let env = TriggerEnvelope::from_composio(
        "notion",
        "NOTION_PAGE_UPDATED",
        "trig-fallback",
        "",
        json!({}),
    );
    assert_eq!(env.external_id, "trig-fallback");
}

#[test]
fn webview_source_has_stable_slug_and_fields() {
    let source = TriggerSource::WebviewIntegration {
        provider: "slack".to_string(),
        account_id: "acct-123".to_string(),
    };
    assert_eq!(source.slug(), "webview");
    match source {
        TriggerSource::WebviewIntegration {
            provider,
            account_id,
        } => {
            assert_eq!(provider, "slack");
            assert_eq!(account_id, "acct-123");
        }
        _ => panic!("expected WebviewIntegration variant"),
    }
}

#[test]
fn webhook_envelope_builds_expected_label_and_slug() {
    let env = TriggerEnvelope::from_webhook(
        "tunnel-uuid-1",
        "POST",
        "/hooks/test",
        json!({ "event": "push" }),
    );
    assert_eq!(env.display_label, "webhook/POST//hooks/test");
    assert_eq!(env.external_id, "tunnel-uuid-1");
    assert_eq!(env.source.slug(), "webhook");
    match env.source {
        TriggerSource::Webhook {
            tunnel_id,
            method,
            path,
        } => {
            assert_eq!(tunnel_id, "tunnel-uuid-1");
            assert_eq!(method, "POST");
            assert_eq!(path, "/hooks/test");
        }
        _ => panic!("expected Webhook variant"),
    }
    assert_eq!(env.payload["event"], "push");
}

#[test]
fn cron_envelope_builds_expected_label_and_slug() {
    let env = TriggerEnvelope::from_cron("job-1", "morning_briefing", "Briefing complete");
    assert_eq!(env.display_label, "cron/morning_briefing");
    assert_eq!(env.external_id, "job-1");
    assert_eq!(env.source.slug(), "cron");
    match env.source {
        TriggerSource::Cron { job_id, job_name } => {
            assert_eq!(job_id, "job-1");
            assert_eq!(job_name, "morning_briefing");
        }
        _ => panic!("expected Cron variant"),
    }
    assert_eq!(env.payload["output"], "Briefing complete");
}

#[test]
fn external_envelope_builds_expected_label_and_slug() {
    let env = TriggerEnvelope::from_external("caller-abc", "ci_pipeline", json!({ "ref": "main" }));
    assert_eq!(env.display_label, "external/caller-abc");
    assert_eq!(env.external_id, "caller-abc");
    assert_eq!(env.source.slug(), "external");
    match env.source {
        TriggerSource::External { caller_id, reason } => {
            assert_eq!(caller_id, "caller-abc");
            assert_eq!(reason, "ci_pipeline");
        }
        _ => panic!("expected External variant"),
    }
    assert_eq!(env.payload["ref"], "main");
}
