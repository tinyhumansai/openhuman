use std::future::Future;

/// Returns whether TinySecurity removed credentials from a flow-memory value.
/// A changed value is rejected by the caller; a module error is also a refusal.
pub(crate) async fn credentials_removed<F, E>(original: &str, answer: F) -> Result<bool, E>
where
    F: Future<Output = Result<String, E>>,
{
    Ok(answer.await? != original)
}

/// Checks credentials through the released TinySecurity module. An unavailable
/// module is an error so callers can fail closed before approvals or writes.
pub(crate) async fn contains_credentials(
    config: &crate::config::Config,
    content: &str,
) -> Result<bool, String> {
    #[cfg(feature = "security-module")]
    {
        credentials_removed(
            content,
            crate::modules::security::text::redact_credentials_with(config, content, &[]),
        )
        .await
    }

    #[cfg(not(feature = "security-module"))]
    {
        let _ = (config, content);
        Err("TinySecurity credential scan is unavailable".to_string())
    }
}

#[cfg(test)]
#[path = "memory_guard_tests.rs"]
mod tests;
