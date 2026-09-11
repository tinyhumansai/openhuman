#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClassifiedErrorSnapshot {
    pub error_type: &'static str,
    pub message: String,
    pub source: &'static str,
    pub retryable: bool,
    pub retry_after_ms: Option<u64>,
    pub provider: Option<String>,
    pub fallback_available: Option<bool>,
}

pub fn classify_error_for_test(err: &str) -> ClassifiedErrorSnapshot {
    let classified = super::classify_inference_error(err);
    ClassifiedErrorSnapshot {
        error_type: classified.error_type,
        message: classified.message,
        source: classified.source,
        retryable: classified.retryable,
        retry_after_ms: classified.retry_after_ms,
        provider: classified.provider,
        fallback_available: classified.fallback_available,
    }
}

pub fn extracted_provider_detail_for_test(err: &str) -> Option<String> {
    super::extract_provider_error_detail(err)
}

pub fn retry_after_secs_for_test(err: &str) -> Option<u64> {
    super::parse_retry_after_secs_from_str(err)
}

pub fn is_non_retryable_rate_limit_for_test(lower: &str) -> bool {
    super::is_non_retryable_rate_limit_text(lower)
}

pub fn key_for_test(thread_id: &str) -> String {
    super::key_for(thread_id)
}

pub fn event_session_id_for_test(client_id: &str, thread_id: &str) -> String {
    super::event_session_id_for(client_id, thread_id)
}

pub async fn set_forced_run_chat_task_error_for_test(message: Option<&str>) {
    super::set_test_forced_run_chat_task_error(message).await;
}
