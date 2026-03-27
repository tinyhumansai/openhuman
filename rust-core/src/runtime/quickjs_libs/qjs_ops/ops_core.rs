//! Core ops: console, crypto, performance, platform, timers.

use parking_lot::RwLock;
use rquickjs::{Ctx, Function, Object};
use std::sync::Arc;
use std::time::{Duration, Instant};

use super::types::{TimerEntry, TimerState, ALLOWED_ENV_VARS};

pub fn register<'js>(
    ctx: &Ctx<'js>,
    ops: &Object<'js>,
    timer_state: Arc<RwLock<TimerState>>,
) -> rquickjs::Result<()> {
    // ========================================================================
    // Console (3)
    // ========================================================================

    ops.set(
        "console_log",
        Function::new(ctx.clone(), |msg: String| {
            log::info!("[js] {}", msg);
        }),
    )?;

    ops.set(
        "console_warn",
        Function::new(ctx.clone(), |msg: String| {
            log::warn!("[js] {}", msg);
        }),
    )?;

    ops.set(
        "console_error",
        Function::new(ctx.clone(), |msg: String| {
            log::error!("[js] {}", msg);
        }),
    )?;

    // ========================================================================
    // Crypto (3)
    // ========================================================================

    ops.set(
        "crypto_random",
        Function::new(ctx.clone(), |len: usize| -> Vec<u8> {
            use rand::RngCore;
            let mut buf = vec![0u8; len];
            rand::thread_rng().fill_bytes(&mut buf);
            buf
        }),
    )?;

    ops.set(
        "atob",
        Function::new(ctx.clone(), |input: String| -> rquickjs::Result<String> {
            use base64::Engine;
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(&input)
                .map_err(|e| super::types::js_err(e.to_string()))?;
            String::from_utf8(bytes).map_err(|e| super::types::js_err(e.to_string()))
        }),
    )?;

    ops.set(
        "btoa",
        Function::new(ctx.clone(), |input: String| -> String {
            use base64::Engine;
            base64::engine::general_purpose::STANDARD.encode(input.as_bytes())
        }),
    )?;

    // ========================================================================
    // Performance (1)
    // ========================================================================

    ops.set(
        "performance_now",
        Function::new(ctx.clone(), || -> f64 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_secs_f64()
                * 1000.0
        }),
    )?;

    // ========================================================================
    // Platform (2)
    // ========================================================================

    ops.set(
        "platform_os",
        Function::new(ctx.clone(), || -> &'static str {
            match std::env::consts::OS {
                "windows" => "windows",
                "macos" => "macos",
                "linux" => "linux",
                "android" => "android",
                "ios" => "ios",
                _ => "unknown",
            }
        }),
    )?;

    ops.set(
        "platform_env",
        Function::new(ctx.clone(), |key: String| -> Option<String> {
            if ALLOWED_ENV_VARS.contains(&key.as_str()) {
                std::env::var(&key).ok()
            } else {
                None
            }
        }),
    )?;

    ops.set(
        "get_session_token",
        Function::new(ctx.clone(), || -> String {
            std::env::var("JWT_TOKEN").unwrap_or_default()
        }),
    )?;

    // ========================================================================
    // Timers (2)
    // ========================================================================

    {
        let ts = timer_state.clone();
        ops.set(
            "timer_start",
            Function::new(
                ctx.clone(),
                move |id: u32, delay_ms: u32, is_interval: bool| {
                    let mut state = ts.write();
                    state.timers.insert(
                        id,
                        TimerEntry {
                            deadline: Instant::now() + Duration::from_millis(delay_ms as u64),
                            delay_ms,
                            is_interval,
                        },
                    );
                },
            ),
        )?;
    }

    {
        let ts = timer_state;
        ops.set(
            "timer_cancel",
            Function::new(ctx.clone(), move |id: u32| {
                let mut state = ts.write();
                state.timers.remove(&id);
            }),
        )?;
    }

    Ok(())
}
