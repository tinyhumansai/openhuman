//! Which days a user turn is about, worked out by one small model call that
//! runs beside the pre-turn recall (`[memory.recall] date_hint`, off by
//! default). Any language: the model reads "kal", "el viernes", "letzte
//! Woche" against the date line, which no keyword list can.
//!
//! The hint only reorders the pack's fetch sections (memories from those
//! days first); a late, failed or unparseable answer leaves the pack as it
//! would have been. Measured through the managed route (2026-10-08): p50
//! ≈ 3 s, so with the 1.5 s default budget most turns finish without it.

use chrono::NaiveDate;
use tinyinference_llm::message::Message;
use tinyinference_llm::model::ModelRequest;
use tinymemory_api::TimeHint;

use crate::config::Config;

const INSTRUCTION: &str = "Return the calendar date range (local dates) that the user's message refers to, as JSON only: {\"from\":\"YYYY-MM-DD\",\"to\":\"YYYY-MM-DD\"}, or {\"from\":null,\"to\":null} when it refers to no date. Resolve relative words (yesterday, last week, a weekday, a month) against the current date. A single day has from == to. If a word can mean either the past or the future (e.g. Hindi/Hinglish 'kal', 'parso'), return a window covering both readings.";

/// The days `user_text` is about, in `zone`, from the user's chat model.
/// `None` when it names no time, or the call fails or answers nonsense.
///
/// Not bounded here: the caller owns the deadline (`hooks::pre_turn` wraps
/// it in the turn's pre-turn timeout), as tinymemory's `pre_turn_dated`
/// requires of its hint.
pub async fn extract(
    ctx: &crate::core::runtime::DispatchCtx,
    config: &Config,
    user_text: &str,
    zone: &str,
) -> Option<TimeHint> {
    let (model, model_id) = match crate::inference::provider::create_chat_model_with_model_id_in(
        ctx, "chat", config, 0.0,
    ) {
        Ok(model) => model,
        Err(error) => {
            tracing::debug!(%error, "[memory:date_hint] no chat model");
            return None;
        }
    };
    let system = format!(
        "{}\n\n{INSTRUCTION}",
        crate::agent::prompts::current_datetime_line(Some(zone))
    );
    let mut request = ModelRequest::new(vec![Message::system(system), Message::user(user_text)]);
    request.max_tokens = Some(400);
    request.temperature = Some(0.0);
    let started = std::time::Instant::now();
    let answer = model.invoke(&(), request).await;
    let elapsed_ms = started.elapsed().as_millis() as u64;
    match answer {
        Ok(response) => {
            let hint = parse(&response.text(), zone);
            tracing::debug!(model = %model_id, elapsed_ms, dated = hint.is_some(), "[memory:date_hint] extracted");
            hint
        }
        Err(error) => {
            tracing::debug!(model = %model_id, elapsed_ms, %error, "[memory:date_hint] call failed");
            None
        }
    }
}

/// The `{"from","to"}` object in a model's answer (code fences and prose
/// around it are tolerated) as a hint in `zone`.
fn parse(answer: &str, zone: &str) -> Option<TimeHint> {
    #[derive(serde::Deserialize)]
    struct Range {
        from: Option<NaiveDate>,
        to: Option<NaiveDate>,
    }
    let json = answer.get(answer.find('{')?..=answer.rfind('}')?)?;
    let range: Range = serde_json::from_str(json).ok()?;
    let from = range.from?;
    TimeHint::new(from, range.to.unwrap_or(from), Some(zone.to_string())).ok()
}

#[cfg(test)]
#[path = "date_hint_tests.rs"]
mod tests;
