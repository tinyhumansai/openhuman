//! Acceptance mirrors for #6027 (channel turns carry the active personality)
//! and #6028 (identity edits reach the next channel turn without a restart).
//!
//! The first test is the channel twin of
//! `build_session_agent_injects_profile_soul_into_prompt` in the session
//! builder tests: the same profile/soul setup, asserted against the prompt a
//! channel turn is seeded with. The second drives the real dispatch path
//! twice through `run_dispatch_harness` with one shared prompt and edits
//! `SOUL.md` in between — the reproduction in the issue, as a test.

use super::super::runtime::test_support::{
    run_dispatch_harness, DispatchHarnessOptions, HarnessSystemPrompt,
};
use super::common::make_workspace;
use crate::openhuman::agent::profiles::home::profile_home;
use crate::openhuman::agent::profiles::store::built_in_default_profile;
use crate::openhuman::agent::profiles::AgentProfileStore;
use crate::openhuman::channels::system_prompt::ChannelPromptInputs;
use crate::openhuman::channels::ChannelSystemPrompt;
use std::path::Path;

const ALICE_SOUL: &str = "I am Alice, a meticulous archivist.";
const EDITED_ROOT_SOUL: &str = "# Soul\nBegin every reply with the word ROOTSOUL2.";

fn inputs(workspace_dir: &Path) -> ChannelPromptInputs {
    ChannelPromptInputs {
        workspace_dir: workspace_dir.to_path_buf(),
        model: "test-model".to_string(),
        tool_descs: Vec::new(),
        skills: Vec::new(),
        bootstrap_max_chars: None,
        suffix: String::new(),
    }
}

fn activate_profile(workspace_dir: &Path, id: &str, soul: &str) {
    let mut profile = built_in_default_profile();
    profile.id = id.to_string();
    profile.name = id.to_string();
    profile.built_in = false;
    profile.is_master = false;
    profile.memory_dir_suffix = None;
    let store = AgentProfileStore::new(workspace_dir.to_path_buf());
    store.upsert(profile).expect("upsert profile");
    store.select(id).expect("select profile");
    let home = profile_home(workspace_dir, id);
    std::fs::create_dir_all(&home).expect("profile home");
    std::fs::write(home.join("SOUL.md"), soul).expect("profile SOUL.md");
}

/// #6027 — with a non-default profile active, the channel prompt carries
/// that profile's soul and not the workspace-root one, while the files a
/// profile does not own (IDENTITY.md) still come from the root.
#[test]
fn channel_prompt_inlines_active_profile_soul_instead_of_root() {
    let ws = make_workspace();
    activate_profile(ws.path(), "alice", ALICE_SOUL);

    let prompt = ChannelSystemPrompt::refreshing(inputs(ws.path()));
    let text = prompt.current();

    assert!(
        text.contains(ALICE_SOUL),
        "the channel prompt must include the active profile's SOUL.md"
    );
    assert!(
        !text.contains("Be helpful"),
        "profile SOUL.md must replace, not accompany, workspace-root SOUL.md"
    );
    assert!(
        text.contains("Name: OpenHuman"),
        "IDENTITY.md is not a profile file and stays the root one"
    );
}

/// #6028 — a `SOUL.md` edit made while the runtime is up changes the system
/// message of the next dispatched turn; no restart, no new context.
#[tokio::test]
async fn channel_reply_follows_soul_edit_without_restart() {
    let ws = make_workspace();
    let prompt = ChannelSystemPrompt::refreshing(inputs(ws.path()));

    let first = run_dispatch_harness(DispatchHarnessOptions {
        workspace_dir: Some(ws.path().to_path_buf()),
        system_prompt: HarnessSystemPrompt::new(prompt.clone()),
        content: "hi".to_string(),
        ..Default::default()
    })
    .await;
    assert!(
        first.handler_history_text.contains("Be helpful"),
        "first turn is seeded with the root soul as written at boot"
    );

    std::fs::write(ws.path().join("SOUL.md"), EDITED_ROOT_SOUL).expect("edit SOUL.md");

    let second = run_dispatch_harness(DispatchHarnessOptions {
        workspace_dir: Some(ws.path().to_path_buf()),
        system_prompt: HarnessSystemPrompt::new(prompt.clone()),
        content: "hi again".to_string(),
        ..Default::default()
    })
    .await;
    assert!(
        second.handler_history_text.contains("ROOTSOUL2"),
        "the edited soul must reach the very next turn"
    );
    assert!(
        !second.handler_history_text.contains("Be helpful"),
        "the stale boot-time soul must be gone"
    );
}

/// The fixed variant the other channel tests rely on is untouched by the
/// refresh machinery: its text is what was given, every time.
#[tokio::test]
async fn fixed_prompt_seeds_every_turn_unchanged() {
    let ws = make_workspace();
    let observed = run_dispatch_harness(DispatchHarnessOptions {
        workspace_dir: Some(ws.path().to_path_buf()),
        system_prompt: HarnessSystemPrompt::new(ChannelSystemPrompt::fixed("pinned prompt")),
        ..Default::default()
    })
    .await;
    assert!(observed.handler_history_text.starts_with("pinned prompt"));
    assert!(!observed.handler_history_text.contains("Be helpful"));
}
