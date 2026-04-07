//! Screenshot capture worker — polls foreground context at baseline FPS,
//! captures the active window via `screencapture -l <windowID>`, saves to
//! disk when `keep_screenshots` is set, and sends frames to the vision
//! processing worker via an unbounded channel.

use std::path::PathBuf;
use std::sync::Arc;

use crate::openhuman::accessibility::{capture_screen_image_ref_for_context, foreground_context};
use crate::openhuman::config::Config;

use super::capture::now_ms;
use super::engine::AccessibilityEngine;
use super::helpers::push_ephemeral_frame;
use super::types::CaptureFrame;

/// Main capture loop. Runs until session TTL expires or the session is stopped.
pub(crate) async fn run(engine: Arc<AccessibilityEngine>) {
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(250));
    tracing::debug!("[capture_worker] started");

    loop {
        tick.tick().await;

        // Check TTL.
        let should_stop = {
            let state = engine.inner.lock().await;
            match &state.session {
                Some(session) => now_ms() >= session.expires_at_ms,
                None => {
                    tracing::debug!("[capture_worker] no session, exiting");
                    return;
                }
            }
        };
        if should_stop {
            tracing::debug!("[capture_worker] TTL expired, stopping");
            engine
                .stop_session_internal("ttl_expired".to_string())
                .await;
            return;
        }

        let context = foreground_context();
        let now = now_ms();
        let mut state = engine.inner.lock().await;
        let baseline_ms = (1000.0_f64 / (state.config.baseline_fps.max(0.2) as f64)).round() as i64;
        let screen_monitoring = state.features.screen_monitoring;
        let config = state.config.clone();

        let Some(session) = state.session.as_mut() else {
            return;
        };
        if !screen_monitoring {
            continue;
        }

        let is_allowed = context
            .as_ref()
            .map(|ctx| engine.should_capture_context(ctx, &config))
            .unwrap_or(false);
        if !is_allowed {
            tracing::trace!(
                "[capture_worker] skipped: context blocked by denylist (app={:?})",
                context.as_ref().and_then(|c| c.app_name.as_deref())
            );
            continue;
        }

        let context_changed = match (&session.last_context, &context) {
            (Some(prev), Some(curr)) => !prev.same_as(curr),
            (None, Some(_)) => true,
            _ => false,
        };

        let baseline_due = session
            .last_capture_at_ms
            .map(|last| now - last >= baseline_ms)
            .unwrap_or(true);

        if !(context_changed || baseline_due) {
            continue;
        }

        let reason = if context_changed {
            "event:foreground_changed"
        } else {
            "baseline"
        };

        // Only capture when we have a window ID — never fall back to fullscreen.
        let has_window_id = context.as_ref().and_then(|c| c.window_id).is_some();
        if !has_window_id {
            tracing::debug!(
                "[capture_worker] skipping: no window_id for app={:?}",
                context.as_ref().and_then(|c| c.app_name.as_deref()),
            );
            session.last_context = context;
            continue;
        }

        tracing::debug!(
            "[capture_worker] capturing app={:?} window_id={:?}",
            context.as_ref().and_then(|c| c.app_name.as_deref()),
            context.as_ref().and_then(|c| c.window_id),
        );

        let capture_result = capture_screen_image_ref_for_context(context.as_ref());
        if let Err(ref e) = capture_result {
            tracing::debug!("[capture_worker] capture failed (reason={}): {}", reason, e);
        }

        let frame = CaptureFrame {
            captured_at_ms: now,
            reason: reason.to_string(),
            app_name: context.as_ref().and_then(|c| c.app_name.clone()),
            window_title: context.as_ref().and_then(|c| c.window_title.clone()),
            image_ref: capture_result.ok(),
        };

        // Save to disk immediately so screenshots land without Ollama delay.
        if frame.image_ref.is_some() && config.keep_screenshots {
            let ws = match Config::load_or_init().await {
                Ok(c) => c.workspace_dir.clone(),
                Err(_) => PathBuf::from("."),
            };
            match AccessibilityEngine::save_screenshot_to_disk(&ws, &frame) {
                Ok(path) => {
                    tracing::debug!("[capture_worker] saved: {}", path.display())
                }
                Err(e) => tracing::debug!("[capture_worker] save failed: {e}"),
            }
        }

        push_ephemeral_frame(&mut session.frames, frame.clone());
        session.capture_count = session.capture_count.saturating_add(1);
        session.last_capture_at_ms = Some(now);
        session.last_context = context;

        if frame.image_ref.is_some() && session.vision_enabled {
            if let Some(tx) = session.vision_tx.as_ref() {
                if tx.send(frame).is_ok() {
                    session.vision_queue_depth =
                        session.vision_queue_depth.saturating_add(1);
                    session.vision_state = "queued".to_string();
                }
            }
        }
        state.last_event = Some(reason.to_string());
    }
}
