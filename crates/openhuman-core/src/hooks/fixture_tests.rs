//! A hook engine pinned for tests that need a live `beforeSubmitPrompt` hook.
//!
//! The production engine is process-wide and every `hooks::init` (each
//! `CoreBuilder::build` in a parallel test) reloads it from disk, so a fixture
//! installed there can vanish between its install and its dispatch. Once
//! [`install_prompt_hooks`] runs, [`super::host::engine`] answers with this
//! engine instead, while `reload` keeps writing the production one. Its hooks
//! act only on prompts that start with [`CONTEXT`] or [`BLOCK`], so no other
//! test's prompt meets them.

use std::sync::Arc;

use tinyagents_hooks::command::{HookConfig, HookDefinition, HookEngine, HookEvent};

/// A prompt starting with this gets `EXTRACTED-BY-HOOK` as added context.
pub(crate) const CONTEXT: &str = "ZZ-HOOK-CONTEXT";
/// A prompt starting with this is blocked with `hook says no`.
pub(crate) const BLOCK: &str = "ZZ-HOOK-BLOCK";

static PINNED: tokio::sync::OnceCell<Arc<HookEngine>> = tokio::sync::OnceCell::const_new();

pub(super) fn pinned() -> Option<&'static Arc<HookEngine>> {
    PINNED.get()
}

/// Pin the fixture engine behind a POSIX shell, once per process.
pub(crate) async fn install_prompt_hooks() {
    PINNED
        .get_or_init(|| async {
            let hook = |command: &str, matcher: String| {
                serde_json::from_value::<HookDefinition>(
                    serde_json::json!({ "command": command, "matcher": matcher }),
                )
                .expect("test hook definition")
            };
            let mut config = HookConfig::default();
            config.by_event.insert(
                HookEvent::BeforeSubmitPrompt,
                vec![
                    hook(
                        r#"cat >/dev/null; echo '{"additional_context": "EXTRACTED-BY-HOOK"}'"#,
                        format!("^{CONTEXT}"),
                    ),
                    hook(
                        r#"cat >/dev/null; echo '{"continue": false, "user_message": "hook says no"}'"#,
                        format!("^{BLOCK}"),
                    ),
                ],
            );
            super::shell::install_posix_for_test();
            let engine = Arc::new(HookEngine::new(super::host::environment()));
            engine.install(config).await;
            engine
        })
        .await;
}
