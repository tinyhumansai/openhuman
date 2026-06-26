//! Operator inbox triage and draft engine.

use super::types::*;
use crate::openhuman::util::now_epoch;
use std::collections::HashMap;
use std::sync::Mutex;
use tracing::{debug, info, warn};

/// Maximum triage records before LRU eviction of archived items.
const MAX_RECORDS: usize = 500;

static RECORDS: std::sync::LazyLock<Mutex<HashMap<String, TriageRecord>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub fn triage_message(
    source: MessageSource,
    sender: &str,
    subject: &str,
    body: &str,
) -> TriageRecord {
    let priority = score_priority(subject, body);
    let reason = priority_reason(&priority, subject, body);
    triage_message_with_priority(source, sender, subject, body, priority, &reason)
}

/// Triage with an externally-determined priority (e.g. from LLM classification).
pub fn triage_message_with_priority(
    source: MessageSource,
    sender: &str,
    subject: &str,
    body: &str,
    priority: TriagePriority,
    reason: &str,
) -> TriageRecord {
    let id = uuid_v4();
    let rec = TriageRecord {
        id: id.clone(),
        source,
        sender: sender.into(),
        subject: subject.into(),
        body_preview: body.chars().take(200).collect(),
        priority,
        reason: reason.to_string(),
        proposed_reply: None,
        follow_up_at: None,
        status: TriageStatus::Pending,
        created_at: now_epoch(),
    };
    let mut store = RECORDS.lock().unwrap_or_else(|e| e.into_inner());
    // Evict oldest archived records if at capacity.
    if store.len() >= MAX_RECORDS {
        let oldest = store
            .iter()
            .filter(|(_, r)| r.status == TriageStatus::Archived)
            .min_by_key(|(_, r)| r.created_at)
            .map(|(id, _)| id.clone());
        if let Some(old_id) = oldest {
            warn!(evicted = %old_id, "[operator_inbox] evicting oldest archived record");
            store.remove(&old_id);
        }
    }
    store.insert(id, rec.clone());
    drop(store);
    info!(triage_id = %rec.id, priority = ?rec.priority, "[operator_inbox] message triaged");
    rec
}

pub fn generate_draft(triage_id: &str, tone: ReplyTone) -> Result<DraftReply, String> {
    debug!(triage_id = %triage_id, tone = ?tone, "[operator_inbox] generating draft");
    let mut store = RECORDS.lock().unwrap_or_else(|e| e.into_inner());
    let rec = store
        .get_mut(triage_id)
        .ok_or_else(|| format!("triage not found: {triage_id}"))?;
    let content = match tone {
        ReplyTone::Professional => format!("Thank you for reaching out regarding \"{}\". I've reviewed your message and will follow up shortly.", rec.subject),
        ReplyTone::Casual => format!("Hey! Got your message about \"{}\". Let me look into it and get back to you.", rec.subject),
        ReplyTone::Formal => format!("Dear {},\n\nThank you for your correspondence regarding \"{}\". We acknowledge receipt and will respond in due course.\n\nBest regards", rec.sender, rec.subject),
    };
    rec.proposed_reply = Some(content.clone());
    rec.status = TriageStatus::Drafted;
    let draft = DraftReply {
        id: format!("dr-{}", triage_id.get(3..).unwrap_or(triage_id)),
        triage_id: triage_id.into(),
        content,
        tone,
        created_at: now_epoch(),
    };
    Ok(draft)
}

pub fn schedule_followup(triage_id: &str, follow_up_at: u64) -> Result<TriageRecord, String> {
    let mut store = RECORDS.lock().unwrap_or_else(|e| e.into_inner());
    let rec = store
        .get_mut(triage_id)
        .ok_or_else(|| format!("triage not found: {triage_id}"))?;
    rec.follow_up_at = Some(follow_up_at);
    info!(triage_id = %triage_id, "[operator_inbox] follow-up scheduled");
    Ok(rec.clone())
}

pub fn archive_triage(triage_id: &str) -> Result<TriageRecord, String> {
    let mut store = RECORDS.lock().unwrap_or_else(|e| e.into_inner());
    let rec = store
        .get_mut(triage_id)
        .ok_or_else(|| format!("triage not found: {triage_id}"))?;
    rec.status = TriageStatus::Archived;
    debug!(triage_id = %triage_id, "[operator_inbox] archived");
    Ok(rec.clone())
}

/// Store LLM-generated draft content on a triage record.
pub fn set_draft_content(triage_id: &str, content: &str) {
    if let Some(rec) = RECORDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(triage_id)
    {
        rec.proposed_reply = Some(content.to_string());
        rec.status = TriageStatus::Drafted;
    }
}

pub fn get_triage(triage_id: &str) -> Result<TriageRecord, String> {
    RECORDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(triage_id)
        .cloned()
        .ok_or_else(|| format!("triage not found: {triage_id}"))
}

pub fn list_triage() -> Vec<TriageRecord> {
    RECORDS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .values()
        .cloned()
        .collect()
}

fn score_priority(subject: &str, body: &str) -> TriagePriority {
    let text = format!("{} {}", subject, body).to_lowercase();
    if text.contains("urgent") || text.contains("emergency") || text.contains("critical") {
        TriagePriority::Urgent
    } else if text.contains("asap") || text.contains("deadline") || text.contains("important") {
        TriagePriority::High
    } else if text.contains("question") || text.contains("help") || text.contains("request") {
        TriagePriority::Normal
    } else {
        TriagePriority::Low
    }
}

fn priority_reason(p: &TriagePriority, subject: &str, _body: &str) -> String {
    match p {
        TriagePriority::Urgent => format!("Urgent keywords detected in: {subject}"),
        TriagePriority::High => format!("High-priority keywords in: {subject}"),
        TriagePriority::Normal => "Standard request".into(),
        TriagePriority::Low => "No priority signals detected".into(),
    }
}

fn uuid_v4() -> String {
    format!("oi-{}", crate::openhuman::util::uuid_v4())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn triage_urgent() {
        let r = triage_message(
            MessageSource::Email,
            "alice@x.com",
            "URGENT: server down",
            "Please fix immediately",
        );
        assert_eq!(r.priority, TriagePriority::Urgent);
    }
    #[test]
    fn triage_high() {
        let r = triage_message(
            MessageSource::Chat,
            "bob",
            "Need this ASAP",
            "deadline tomorrow",
        );
        assert_eq!(r.priority, TriagePriority::High);
    }
    #[test]
    fn triage_normal() {
        let r = triage_message(
            MessageSource::Social,
            "carol",
            "Question about setup",
            "Can you help?",
        );
        assert_eq!(r.priority, TriagePriority::Normal);
    }
    #[test]
    fn triage_low() {
        let r = triage_message(
            MessageSource::Webhook,
            "system",
            "Weekly digest",
            "Here are your stats",
        );
        assert_eq!(r.priority, TriagePriority::Low);
    }
    #[test]
    fn generate_draft_professional() {
        let r = triage_message(MessageSource::Email, "x@y.com", "Meeting", "Let's meet");
        let d = generate_draft(&r.id, ReplyTone::Professional).unwrap();
        assert!(d.content.contains("Meeting"));
        assert_eq!(d.triage_id, r.id);
    }
    #[test]
    fn generate_draft_casual() {
        let r = triage_message(MessageSource::Chat, "friend", "Hey", "What's up");
        let d = generate_draft(&r.id, ReplyTone::Casual).unwrap();
        assert!(d.content.contains("Hey"));
    }
    #[test]
    fn generate_draft_not_found() {
        assert!(generate_draft("nope", ReplyTone::Formal).is_err());
    }

    #[test]
    fn generate_draft_short_id_no_panic() {
        // IDs shorter than 3 chars should not panic on slice.
        assert!(generate_draft("ab", ReplyTone::Professional).is_err());
    }
    #[test]
    fn schedule_followup_works() {
        let r = triage_message(MessageSource::Email, "x", "Test", "body");
        let r = schedule_followup(&r.id, 1700000000).unwrap();
        assert_eq!(r.follow_up_at, Some(1700000000));
    }
    #[test]
    fn archive_works() {
        let r = triage_message(MessageSource::Email, "x", "Archive me", "body");
        let r = archive_triage(&r.id).unwrap();
        assert_eq!(r.status, TriageStatus::Archived);
    }
    #[test]
    fn get_triage_works() {
        let r = triage_message(MessageSource::Chat, "x", "Get test", "body");
        assert_eq!(get_triage(&r.id).unwrap().subject, "Get test");
    }
    #[test]
    fn get_triage_not_found() {
        assert!(get_triage("nope").is_err());
    }
    #[test]
    fn list_triage_not_empty() {
        triage_message(MessageSource::Email, "x", "List test", "body");
        assert!(!list_triage().is_empty());
    }
}
