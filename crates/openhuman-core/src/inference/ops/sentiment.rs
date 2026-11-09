use crate::config::Config;
use crate::core::Outcome;
use crate::inference::host_runtime as local_runtime;
use tinyinference_llm::sentiment::{parse_sentiment_response, sentiment_prompt, SentimentResult};
use tracing::{debug, warn};

const LOG_PREFIX: &str = "[inference::ops]";

pub async fn inference_analyze_sentiment(
    config: &Config,
    message: &str,
) -> Result<Outcome<SentimentResult>, String> {
    debug!(
        message_len = message.len(),
        "{LOG_PREFIX} analyze_sentiment:start"
    );
    if message.trim().is_empty() {
        return Ok(Outcome::single_log(
            SentimentResult::neutral(),
            "empty message — neutral sentiment",
        ));
    }

    let service = local_runtime::global(config);
    if service.status().state != "ready" {
        return Ok(Outcome::single_log(
            SentimentResult::neutral(),
            "local model not ready",
        ));
    }

    let prompt = sentiment_prompt(message);
    let runtime = crate::inference::local_runtime_config(config);
    let Some(_permit) = crate::platform::llm_gate::wait_for_capacity().await else {
        return Ok(Outcome::single_log(
            SentimentResult::neutral(),
            "local inference paused while signed out",
        ));
    };
    let result = match service.prompt(&runtime, &prompt, Some(8), true).await {
        Ok(raw) => parse_sentiment_response(&raw.trim().to_lowercase()),
        Err(error) => {
            debug!(%error, "{LOG_PREFIX} sentiment inference failed; returning neutral");
            SentimentResult::neutral()
        }
    };
    let result = Ok(Outcome::single_log(result, "sentiment analysis completed"));
    match &result {
        Ok(outcome) => {
            debug!(valence = %outcome.value.valence, "{LOG_PREFIX} analyze_sentiment:ok")
        }
        Err(err) => warn!(error = %err, "{LOG_PREFIX} analyze_sentiment:error"),
    }
    result
}
