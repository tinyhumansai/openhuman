use std::sync::Arc;

use crate::openhuman::agent::profiles::AgentProfileStore;
use crate::openhuman::config::rpc as config_rpc;
use crate::openhuman::threads::turn_state::TurnStateStore;

use super::ops::{key_for, THREAD_SESSIONS};
use super::progress_bridge::spawn_progress_bridge;
use super::session::{
    build_session_agent, build_session_fingerprint, normalize_model_override, pick_target_agent_id,
    provider_role_for_model_override,
};
use super::types::SessionEntry;
use super::types::{ChatRequestMetadata, WebChatTaskResult};
use super::web_errors::{
    inference_budget_exceeded_user_message, is_inference_budget_exceeded_error,
};

#[cfg(any(test, debug_assertions))]
use super::ops::TEST_FORCED_RUN_CHAT_TASK_ERROR;

pub(crate) async fn run_chat_task(
    client_id: &str,
    thread_id: &str,
    request_id: &str,
    message: &str,
    model_override: Option<String>,
    temperature: Option<f64>,
    profile_id: Option<String>,
    locale: Option<String>,
    run_queue: Arc<crate::openhuman::agent::harness::run_queue::RunQueue>,
    metadata: ChatRequestMetadata,
) -> Result<WebChatTaskResult, String> {
    #[cfg(any(test, debug_assertions))]
    {
        let mut slot = TEST_FORCED_RUN_CHAT_TASK_ERROR.lock().await;
        if let Some(forced) = slot.take() {
            log::debug!(
                "[web-channel][test] forced run_chat_task failure client_id={} thread_id={} request_id={}",
                client_id,
                thread_id,
                request_id
            );
            return Err(forced);
        }
    }

    let config = config_rpc::load_config_with_timeout().await?;
    let (_profiles_state, profile) =
        AgentProfileStore::new(config.workspace_dir.clone()).resolve(profile_id.as_deref())?;
    let map_key = key_for(thread_id);
    let model_override = normalize_model_override(profile.model_override.clone())
        .or_else(|| normalize_model_override(model_override));
    let temperature = profile.temperature.or(temperature);
    let target_agent_id = pick_target_agent_id(&config, &profile);
    let provider_role = provider_role_for_model_override(model_override.as_deref());
    let current_fp = build_session_fingerprint(
        &config,
        model_override.clone(),
        temperature,
        target_agent_id.clone(),
        provider_role,
    );

    let prior = {
        let mut sessions = THREAD_SESSIONS.lock().await;
        sessions.remove(&map_key)
    };

    let (mut agent, was_built_fresh) = match prior {
        Some(entry) if entry.fingerprint == current_fp => {
            log::info!(
                "[web-channel] reusing cached session agent id={} for client={} thread={}",
                target_agent_id,
                client_id,
                thread_id
            );
            (entry.agent, false)
        }
        Some(prior_entry) => {
            log::info!(
                "[web-channel] cache miss — rebuilding session agent \
                 (was id={}, now id={}; prior_provider_binding={}, now={}) \
                 for client={} thread={}",
                prior_entry.fingerprint.target_agent_id,
                target_agent_id,
                prior_entry.fingerprint.provider_binding,
                current_fp.provider_binding,
                client_id,
                thread_id
            );
            (
                build_session_agent(
                    &config,
                    client_id,
                    thread_id,
                    &target_agent_id,
                    &profile,
                    model_override.clone(),
                    temperature,
                    locale.as_deref(),
                )?,
                true,
            )
        }
        None => (
            build_session_agent(
                &config,
                client_id,
                thread_id,
                &target_agent_id,
                &profile,
                model_override.clone(),
                temperature,
                locale.as_deref(),
            )?,
            true,
        ),
    };

    // Cold-boot resume from the conversation JSONL.
    if was_built_fresh {
        match crate::openhuman::memory_conversations::get_messages(
            config.workspace_dir.clone(),
            thread_id,
        ) {
            Ok(prior_messages) if !prior_messages.is_empty() => {
                let pairs: Vec<(String, String)> = prior_messages
                    .into_iter()
                    .map(|m| (m.sender, m.content))
                    .collect();
                if let Err(err) = agent.seed_resume_from_messages(pairs, message) {
                    log::warn!(
                        "[web-channel] failed to seed agent resume from conversation log \
                         thread={} err={}",
                        thread_id,
                        err
                    );
                }
            }
            Ok(_) => {
                log::debug!(
                    "[web-channel] no prior messages to seed for thread={} — first turn",
                    thread_id
                );
            }
            Err(err) => {
                log::warn!(
                    "[web-channel] failed to read conversation log for resume thread={} err={}",
                    thread_id,
                    err
                );
            }
        }
    }

    let (progress_tx, progress_rx) = tokio::sync::mpsc::channel(64);
    agent.set_on_progress(Some(progress_tx));
    agent.set_run_queue(Some(run_queue));
    let turn_state_store = TurnStateStore::new(config.workspace_dir.clone());
    spawn_progress_bridge(
        progress_rx,
        client_id.to_string(),
        thread_id.to_string(),
        request_id.to_string(),
        turn_state_store,
        metadata.clone(),
        config.clone(),
    );

    let result = match crate::openhuman::inference::provider::thread_context::with_thread_id(
        thread_id.to_string(),
        agent.run_single(message),
    )
    .await
    {
        Ok(response) => {
            let citations = agent.take_last_turn_citations();
            Ok(WebChatTaskResult {
                full_response: response,
                citations,
            })
        }
        Err(err) => {
            let err_message = err.to_string();
            if is_inference_budget_exceeded_error(&err_message) {
                log::warn!(
                    "[web-channel] inference budget exhausted for client={} thread={} request_id={} error_category=budget_exhausted",
                    client_id,
                    thread_id,
                    request_id
                );
                Ok(WebChatTaskResult {
                    full_response: inference_budget_exceeded_user_message().to_string(),
                    citations: Vec::new(),
                })
            } else {
                Err(err_message)
            }
        }
    };

    if let Ok(ref task_result) = result {
        let speak_reply = matches!(metadata.speak_reply, Some(true));
        let trimmed_response = task_result.full_response.trim();
        if speak_reply && !trimmed_response.is_empty() {
            let opts = crate::openhuman::voice::reply_speech::ReplySpeechOptions::default();
            match crate::openhuman::voice::reply_speech::synthesize_reply(
                &config,
                &task_result.full_response,
                &opts,
            )
            .await
            {
                Ok(_) => log::debug!(
                    "[web_channel] reply_speech dispatched chars={} client_id={} thread_id={} request_id={}",
                    task_result.full_response.len(),
                    client_id,
                    thread_id,
                    request_id,
                ),
                Err(err) => log::warn!(
                    "[web_channel] reply_speech failed: {err} client_id={} thread_id={} request_id={}",
                    client_id,
                    thread_id,
                    request_id,
                ),
            }
        }
        if metadata.source.as_deref() == Some("ptt") {
            if let Some(session_id) = metadata.session_id {
                crate::openhuman::voice::publish_ptt_transcript_committed(
                    thread_id.to_string(),
                    session_id,
                    task_result.full_response.chars().count(),
                    0,
                    false,
                );
            }
        }
    }

    agent.set_on_progress(None);

    {
        let mut sessions = THREAD_SESSIONS.lock().await;
        sessions.insert(
            map_key,
            SessionEntry {
                agent,
                fingerprint: current_fp,
            },
        );
    }

    result
}
