#[cfg(target_os = "macos")]
use super::*;
#[cfg(target_os = "macos")]
use crate::openhuman::screen_intelligence::state::EngineState;
#[cfg(target_os = "macos")]
use tokio::sync::Mutex;
#[cfg(target_os = "macos")]
use tokio::time::Duration;

#[cfg(target_os = "macos")]
#[tokio::test]
async fn enable_with_existing_session_does_not_deadlock() {
    let engine = Arc::new(AccessibilityEngine {
        inner: Mutex::new(EngineState::new(ScreenIntelligenceConfig {
            enabled: true,
            ..Default::default()
        })),
    });

    {
        let mut state = engine.inner.lock().await;
        state.session = Some(new_session_runtime(&state.config, now_ms(), i64::MAX, 300));
    }

    let result = tokio::time::timeout(Duration::from_millis(250), engine.enable()).await;
    assert!(
        result.is_ok(),
        "enable should not deadlock with an active session"
    );
    assert!(
        result.unwrap().is_ok(),
        "enable should return the existing session status"
    );
}
