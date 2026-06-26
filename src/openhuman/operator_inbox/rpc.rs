//! RPC handlers for operator_inbox domain.
use super::{engine, types::*};
use serde_json::{json, Map, Value};
use tracing::debug;

pub async fn handle_triage_message(p: Map<String, Value>) -> Result<Value, String> {
    debug!("[operator_inbox] triage_message RPC entry");
    let source = match p.get("source").and_then(|v| v.as_str()).unwrap_or("email") {
        "chat" => MessageSource::Chat,
        "social" => MessageSource::Social,
        "webhook" => MessageSource::Webhook,
        _ => MessageSource::Email,
    };
    let sender = p.get("sender").and_then(|v| v.as_str()).unwrap_or("");
    let subject = p.get("subject").and_then(|v| v.as_str()).unwrap_or("");
    let body = p.get("body").and_then(|v| v.as_str()).unwrap_or("");

    // Reject messages where all content fields are empty.
    if sender.is_empty() && subject.is_empty() && body.is_empty() {
        return Ok(
            json!({"ok": false, "error": "at least one of sender, subject, or body is required"}),
        );
    }

    // Primary path: LLM-powered triage for intelligent prioritization.
    if let Some((priority, reason)) = try_llm_triage(sender, subject, body).await {
        let r =
            engine::triage_message_with_priority(source, sender, subject, body, priority, &reason);
        return Ok(
            json!({"ok":true,"triage_id":r.id,"priority":r.priority,"reason":r.reason,"status":r.status,"source":"llm"}),
        );
    }

    // Fallback: keyword-based triage.
    let r = engine::triage_message(source, sender, subject, body);
    Ok(
        json!({"ok":true,"triage_id":r.id,"priority":r.priority,"reason":r.reason,"status":r.status,"source":"keyword"}),
    )
}

/// LLM-powered priority classification for incoming messages.
async fn try_llm_triage(
    sender: &str,
    subject: &str,
    body: &str,
) -> Option<(TriagePriority, String)> {
    use crate::openhuman::config::ops::load_config_with_timeout;
    use crate::openhuman::inference::provider::create_chat_provider;

    let config = load_config_with_timeout().await.ok()?;
    let (provider, model) = create_chat_provider("agentic", &config).ok()?;

    let prompt = format!(
        "Classify this message's priority and explain why in one sentence.\n\nFrom: {}\nSubject: {}\nBody: {}\n\nRespond with ONLY valid JSON:\n{{\"priority\": \"urgent|high|normal|low\", \"reason\": \"<one sentence>\"}}",
        sender, subject, &body.chars().take(500).collect::<String>()
    );

    let system = "You are an email triage assistant. Classify message priority based on urgency, sender importance, and content. Be concise.";

    let text = provider
        .chat_with_system(Some(system), &prompt, &model, 0.2)
        .await
        .ok()?;

    // Parse LLM response.
    let trimmed = text.trim();
    let json_str = if let Some(start) = trimmed.find('{') {
        if let Some(end) = trimmed.rfind('}') {
            &trimmed[start..=end]
        } else {
            return None;
        }
    } else {
        return None;
    };

    let parsed: serde_json::Value = serde_json::from_str(json_str).ok()?;
    let priority = match parsed.get("priority")?.as_str()? {
        "urgent" => TriagePriority::Urgent,
        "high" => TriagePriority::High,
        "normal" => TriagePriority::Normal,
        _ => TriagePriority::Low,
    };
    let reason = parsed.get("reason")?.as_str()?.to_string();

    debug!(priority = ?priority, "[operator_inbox] LLM triage complete");
    Some((priority, reason))
}

pub async fn handle_generate_draft(p: Map<String, Value>) -> Result<Value, String> {
    let id = p.get("triage_id").and_then(|v| v.as_str()).unwrap_or("");
    if id.is_empty() {
        return Ok(json!({"ok": false, "error": "triage_id is required"}));
    }
    let tone = match p
        .get("tone")
        .and_then(|v| v.as_str())
        .unwrap_or("professional")
    {
        "casual" => ReplyTone::Casual,
        "formal" => ReplyTone::Formal,
        _ => ReplyTone::Professional,
    };

    // Try LLM-powered draft generation first.
    let rec = engine::get_triage(id)?;
    let llm_content = try_llm_draft(&rec, &tone).await;

    match llm_content {
        Some(content) => {
            // LLM succeeded — store the draft.
            let draft_id = format!("dr-{}", id.get(3..).unwrap_or(id));
            engine::set_draft_content(id, &content);
            Ok(json!({"ok":true,"draft_id":draft_id,"content":content,"tone":tone,"source":"llm"}))
        }
        None => {
            // Fallback to template-based draft.
            match engine::generate_draft(id, tone) {
                Ok(d) => Ok(
                    json!({"ok":true,"draft_id":d.id,"content":d.content,"tone":d.tone,"source":"template"}),
                ),
                Err(e) => Ok(json!({"ok":false,"error":e})),
            }
        }
    }
}

/// Attempt LLM-powered draft generation. Returns None if LLM unavailable.
async fn try_llm_draft(rec: &TriageRecord, tone: &ReplyTone) -> Option<String> {
    use crate::openhuman::config::ops::load_config_with_timeout;
    use crate::openhuman::inference::provider::create_chat_provider;

    let config = load_config_with_timeout().await.ok()?;
    let (provider, model) = create_chat_provider("agentic", &config).ok()?;

    let tone_str = match tone {
        ReplyTone::Professional => "professional",
        ReplyTone::Casual => "casual",
        ReplyTone::Formal => "formal",
    };

    let prompt = format!(
        "Write a {} reply to this email. Be concise. Do not include subject line or headers.\n\nFrom: {}\nSubject: {}\nBody: {}\n\nReply:",
        tone_str, rec.sender, rec.subject, rec.body_preview
    );

    let system = "You are a professional email assistant. Write concise, contextual replies.";

    let text = provider
        .chat_with_system(Some(system), &prompt, &model, 0.6)
        .await
        .ok()?;

    debug!(triage_id = %rec.id, "[operator_inbox] LLM draft generated");
    Some(text)
}

pub async fn handle_schedule_followup(p: Map<String, Value>) -> Result<Value, String> {
    let id = p.get("triage_id").and_then(|v| v.as_str()).unwrap_or("");
    if id.is_empty() {
        return Ok(json!({"ok": false, "error": "triage_id is required"}));
    }
    let at = p.get("follow_up_at").and_then(|v| v.as_u64()).unwrap_or(0);
    if at == 0 {
        return Ok(
            json!({"ok": false, "error": "follow_up_at must be a non-zero epoch timestamp"}),
        );
    }
    match engine::schedule_followup(id, at) {
        Ok(r) => Ok(json!({"ok":true,"triage_id":r.id,"follow_up_at":r.follow_up_at})),
        Err(e) => Ok(json!({"ok":false,"error":e})),
    }
}

pub async fn handle_get_triage(p: Map<String, Value>) -> Result<Value, String> {
    let id = p.get("triage_id").and_then(|v| v.as_str()).unwrap_or("");
    match engine::get_triage(id) {
        Ok(r) => Ok(
            json!({"ok":true,"triage_id":r.id,"priority":r.priority,"status":r.status,"subject":r.subject}),
        ),
        Err(e) => Ok(json!({"ok":false,"error":e})),
    }
}

pub async fn handle_list_triage(_p: Map<String, Value>) -> Result<Value, String> {
    let all: Vec<Value> = engine::list_triage()
        .iter()
        .map(|r| json!({"id":r.id,"priority":r.priority,"status":r.status,"subject":r.subject}))
        .collect();
    Ok(json!({"ok":true,"records":all}))
}

pub async fn handle_archive(p: Map<String, Value>) -> Result<Value, String> {
    let id = p.get("triage_id").and_then(|v| v.as_str()).unwrap_or("");
    match engine::archive_triage(id) {
        Ok(r) => Ok(json!({"ok":true,"triage_id":r.id,"status":r.status})),
        Err(e) => Ok(json!({"ok":false,"error":e})),
    }
}

pub async fn handle_fetch_inbox(p: Map<String, Value>) -> Result<Value, String> {
    let host = p.get("host").and_then(|v| v.as_str()).unwrap_or("");
    let port = p.get("port").and_then(|v| v.as_u64()).unwrap_or(993) as u16;
    let username = p.get("username").and_then(|v| v.as_str()).unwrap_or("");
    let password = p.get("password").and_then(|v| v.as_str()).unwrap_or("");
    let mailbox = p.get("mailbox").and_then(|v| v.as_str()).unwrap_or("INBOX");

    if host.is_empty() || username.is_empty() || password.is_empty() {
        return Ok(json!({"ok": false, "error": "host, username, and password are required"}));
    }

    let config = super::imap_client::ImapConfig {
        host: host.into(),
        port,
        username: username.into(),
        password: password.into(),
        use_tls: true,
        mailbox: mailbox.into(),
        oauth2_token: None,
    };

    match super::connection::fetch_new_emails(&config).await {
        Ok(result) => {
            let triaged: Vec<Value> = result
                .emails
                .iter()
                .map(|email| {
                    let r = engine::triage_message(
                        super::types::MessageSource::Email,
                        &email.from,
                        &email.subject,
                        &email.body_text,
                    );
                    json!({"triage_id": r.id, "priority": r.priority, "subject": r.subject})
                })
                .collect();
            Ok(json!({"ok": true, "fetched": result.new_count, "triaged": triaged}))
        }
        Err(e) => Ok(json!({"ok": false, "error": e})),
    }
}

pub async fn handle_send_reply(p: Map<String, Value>) -> Result<Value, String> {
    let triage_id = p.get("triage_id").and_then(|v| v.as_str()).unwrap_or("");
    let smtp_host = p.get("smtp_host").and_then(|v| v.as_str()).unwrap_or("");
    let smtp_port = p.get("smtp_port").and_then(|v| v.as_u64()).unwrap_or(587) as u16;
    let username = p.get("username").and_then(|v| v.as_str()).unwrap_or("");
    let password = p.get("password").and_then(|v| v.as_str()).unwrap_or("");
    let from = p.get("from").and_then(|v| v.as_str()).unwrap_or("");

    if triage_id.is_empty() || smtp_host.is_empty() || from.is_empty() {
        return Ok(json!({"ok": false, "error": "triage_id, smtp_host, and from are required"}));
    }

    // Get the triage record and its draft.
    let rec = engine::get_triage(triage_id)?;
    let content = rec
        .proposed_reply
        .ok_or_else(|| "no draft generated for this triage".to_string())?;

    let config = super::imap_client::SmtpConfig {
        host: smtp_host.into(),
        port: smtp_port,
        username: username.into(),
        password: password.into(),
        use_tls: true,
        from_address: from.into(),
        from_name: String::new(),
    };

    match super::connection::send_reply(&config, &rec.sender, &rec.subject, &content).await {
        Ok(()) => Ok(json!({"ok": true, "message_id": format!("sent-{}", triage_id)})),
        Err(e) => Ok(json!({"ok": false, "error": e})),
    }
}

pub async fn handle_start_poller(p: Map<String, Value>) -> Result<Value, String> {
    let host = p.get("host").and_then(|v| v.as_str()).unwrap_or("");
    let username = p.get("username").and_then(|v| v.as_str()).unwrap_or("");
    let password = p.get("password").and_then(|v| v.as_str()).unwrap_or("");
    let interval = p.get("interval_secs").and_then(|v| v.as_u64());

    if host.is_empty() || username.is_empty() || password.is_empty() {
        return Ok(json!({"ok": false, "error": "host, username, password required"}));
    }

    let config = super::imap_client::ImapConfig {
        host: host.into(),
        port: 993,
        username: username.into(),
        password: password.into(),
        use_tls: true,
        mailbox: "INBOX".into(),
        oauth2_token: None,
    };

    let started = super::poller::start_polling(config, interval);
    Ok(json!({"ok": true, "started": started}))
}

pub async fn handle_stop_poller(_p: Map<String, Value>) -> Result<Value, String> {
    let was_running = super::poller::is_polling();
    super::poller::stop_polling();
    Ok(json!({"ok": true, "was_running": was_running}))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn triage_rpc() {
        let mut p = Map::new();
        p.insert("sender".into(), Value::String("test@x.com".into()));
        p.insert("subject".into(), Value::String("URGENT help".into()));
        p.insert("body".into(), Value::String("Need help now".into()));
        let r = handle_triage_message(p).await.unwrap();
        assert_eq!(r["ok"], true);
        assert_eq!(r["priority"], "urgent");
    }
    #[tokio::test]
    async fn list_rpc() {
        let r = handle_list_triage(Map::new()).await.unwrap();
        assert_eq!(r["ok"], true);
    }
}
