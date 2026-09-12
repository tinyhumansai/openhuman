use super::*;
use crate::openhuman::agent::profiles::home::profile_home;
use crate::openhuman::agent::profiles::store::built_in_default_profile;
use tempfile::TempDir;

const ROOT_SOUL: &str = "# Soul\nBe helpful.";
const EDITED_ROOT_SOUL: &str = "# Soul\nBegin every reply with the word ROOTSOUL2.";
const ALICE_SOUL: &str = "I am Alice, a meticulous archivist.";

fn workspace() -> TempDir {
    let tmp = TempDir::new().expect("tempdir");
    std::fs::write(tmp.path().join("SOUL.md"), ROOT_SOUL).expect("SOUL.md");
    std::fs::write(tmp.path().join("IDENTITY.md"), "Name: OpenHuman").expect("IDENTITY.md");
    tmp
}

fn inputs(workspace_dir: &Path) -> ChannelPromptInputs {
    ChannelPromptInputs {
        workspace_dir: workspace_dir.to_path_buf(),
        model: "test-model".to_string(),
        tool_descs: vec![("shell".to_string(), "Run commands".to_string())],
        skills: Vec::new(),
        bootstrap_max_chars: None,
        suffix: "\n## Suffix\n\nstatic tail\n".to_string(),
    }
}

/// Creates a non-default profile the way the desktop UI does — through the
/// store — selects it, and gives it its own soul under its profile home.
fn activate_profile(workspace_dir: &Path, id: &str, soul: &str) {
    activate_profile_with(workspace_dir, id, soul, |_| {});
}

/// Like [`activate_profile`], with a hook to adjust the record before it is
/// stored (a `system_prompt_suffix`, a `soul_md_path`, …).
fn activate_profile_with(
    workspace_dir: &Path,
    id: &str,
    soul: &str,
    adjust: impl FnOnce(&mut crate::openhuman::agent::profiles::AgentProfile),
) {
    let mut profile = built_in_default_profile();
    profile.id = id.to_string();
    profile.name = id.to_string();
    profile.built_in = false;
    profile.is_master = false;
    profile.memory_dir_suffix = None;
    adjust(&mut profile);
    let store = AgentProfileStore::new(workspace_dir.to_path_buf());
    store.upsert(profile).expect("upsert profile");
    store.select(id).expect("select profile");
    let home = profile_home(workspace_dir, id);
    std::fs::create_dir_all(&home).expect("profile home");
    std::fs::write(home.join("SOUL.md"), soul).expect("profile SOUL.md");
}

#[test]
fn fixed_prompt_returns_the_same_text_every_time() {
    let prompt = ChannelSystemPrompt::fixed("system prompt");
    let first = prompt.current();
    let second = prompt.current();
    assert!(
        Arc::ptr_eq(&first, &second),
        "a fixed prompt is never re-rendered"
    );
    assert_eq!(first.as_str(), "system prompt");
}

#[test]
fn refreshing_prompt_renders_root_identity_and_suffix_at_construction() {
    let ws = workspace();
    let prompt = ChannelSystemPrompt::refreshing(inputs(ws.path()));
    let text = prompt.current();
    assert!(text.contains("Be helpful"), "root SOUL.md inlined");
    assert!(text.contains("Name: OpenHuman"), "root IDENTITY.md inlined");
    assert!(text.contains("**shell**"), "tool descriptions rendered");
    assert!(text.contains("static tail"), "suffix appended verbatim");
}

#[test]
fn unchanged_identity_hands_back_the_same_arc() {
    let ws = workspace();
    let prompt = ChannelSystemPrompt::refreshing(inputs(ws.path()));
    let first = prompt.current();
    let second = prompt.current();
    assert!(
        Arc::ptr_eq(&first, &second),
        "no identity change must mean no re-render (prefix-cache stability)"
    );
}

#[test]
fn clones_share_one_cache() {
    let ws = workspace();
    let prompt = ChannelSystemPrompt::refreshing(inputs(ws.path()));
    let twin = prompt.clone();
    assert!(Arc::ptr_eq(&prompt.current(), &twin.current()));
}

#[test]
fn root_soul_edit_is_picked_up_without_restart() {
    let ws = workspace();
    let prompt = ChannelSystemPrompt::refreshing(inputs(ws.path()));
    let before = prompt.current();
    assert!(before.contains("Be helpful"));

    std::fs::write(ws.path().join("SOUL.md"), EDITED_ROOT_SOUL).expect("edit SOUL.md");
    let after = prompt.current();
    assert!(!Arc::ptr_eq(&before, &after), "an edit must re-render");
    assert!(
        after.contains("ROOTSOUL2"),
        "the edited soul is what renders"
    );
    assert!(!after.contains("Be helpful"), "the stale soul is gone");

    let settled = prompt.current();
    assert!(
        Arc::ptr_eq(&after, &settled),
        "once re-rendered the prompt is stable again"
    );
}

#[test]
fn profile_switch_replaces_root_soul_with_profile_soul() {
    let ws = workspace();
    let prompt = ChannelSystemPrompt::refreshing(inputs(ws.path()));
    assert!(prompt.current().contains("Be helpful"));

    activate_profile(ws.path(), "alice", ALICE_SOUL);
    let text = prompt.current();
    assert!(
        text.contains(ALICE_SOUL),
        "the active profile's soul renders"
    );
    assert!(
        !text.contains("Be helpful"),
        "the profile soul replaces the root soul rather than joining it"
    );
    assert!(
        text.contains("Name: OpenHuman"),
        "IDENTITY.md stays the root file"
    );

    let settled = prompt.current();
    assert!(Arc::ptr_eq(&text, &settled), "stable after the switch");
}

#[test]
fn profile_soul_edit_is_picked_up_without_restart() {
    let ws = workspace();
    activate_profile(ws.path(), "alice", ALICE_SOUL);
    let prompt = ChannelSystemPrompt::refreshing(inputs(ws.path()));
    assert!(prompt.current().contains(ALICE_SOUL));

    std::fs::write(
        profile_home(ws.path(), "alice").join("SOUL.md"),
        "I am Alice, and today I am brief.",
    )
    .expect("edit profile SOUL.md");
    let text = prompt.current();
    assert!(text.contains("today I am brief"));
    assert!(!text.contains("meticulous archivist"));
}

#[test]
fn root_memory_written_after_boot_is_included() {
    let ws = workspace();
    let prompt = ChannelSystemPrompt::refreshing(inputs(ws.path()));
    assert!(
        !prompt.current().contains("### MEMORY.md"),
        "fresh install: no MEMORY.md"
    );

    std::fs::write(ws.path().join("MEMORY.md"), "# Memory\nUser likes Rust.").expect("MEMORY.md");
    let text = prompt.current();
    assert!(text.contains("### MEMORY.md"));
    assert!(text.contains("User likes Rust."));
}

#[test]
fn unreadable_profile_store_falls_back_to_root_identity() {
    let ws = workspace();
    std::fs::write(ws.path().join("agent_profiles.json"), "{ this is not json").expect("corrupt");
    let prompt = ChannelSystemPrompt::refreshing(inputs(ws.path()));
    let text = prompt.current();
    assert!(
        text.contains("Be helpful"),
        "root soul renders when the store is unreadable"
    );
    assert_eq!(
        resolve_channel_identity_once(ws.path(), &AtomicBool::new(false)).profile_id,
        DEFAULT_PROFILE_ID
    );
}

#[test]
fn identity_fingerprint_distinguishes_missing_from_present() {
    let ws = workspace();
    let before = identity_fingerprint(ws.path(), DEFAULT_PROFILE_ID, None);
    std::fs::write(ws.path().join("PROFILE.md"), "Name: Test User").expect("PROFILE.md");
    let after = identity_fingerprint(ws.path(), DEFAULT_PROFILE_ID, None);
    assert_ne!(before, after, "a file appearing must move the fingerprint");
    assert_eq!(
        after,
        identity_fingerprint(ws.path(), DEFAULT_PROFILE_ID, None),
        "the fingerprint is deterministic for an unchanged tree"
    );
}

#[test]
fn identity_fingerprint_watches_the_profile_store_file() {
    let ws = workspace();
    let before = identity_fingerprint(ws.path(), DEFAULT_PROFILE_ID, None);
    activate_profile(ws.path(), "alice", ALICE_SOUL);
    let after = identity_fingerprint(ws.path(), DEFAULT_PROFILE_ID, None);
    assert_ne!(
        before, after,
        "writing through AgentProfileStore must touch the watched file"
    );
}

#[test]
fn debug_output_names_the_variant_without_leaking_the_prompt() {
    let ws = workspace();
    let fixed = format!("{:?}", ChannelSystemPrompt::fixed("secret soul text"));
    assert!(fixed.contains("Fixed"));
    assert!(!fixed.contains("secret soul text"));
    let refreshing = format!("{:?}", ChannelSystemPrompt::refreshing(inputs(ws.path())));
    assert!(refreshing.contains("Refreshing"));
    assert!(refreshing.contains(DEFAULT_PROFILE_ID));
    assert!(!refreshing.contains("Be helpful"));
}

#[test]
fn profile_prompt_suffix_renders_as_the_desktop_agent_profile_block() {
    let ws = workspace();
    let prompt = ChannelSystemPrompt::refreshing(inputs(ws.path()));
    assert!(!prompt.current().contains("## Agent profile"));

    activate_profile_with(ws.path(), "haiku", ALICE_SOUL, |profile| {
        profile.system_prompt_suffix = Some("  Always answer in haiku.  ".to_string());
    });
    let text = prompt.current();
    let block = crate::openhuman::agent::profiles::prompt_section::render_agent_profile_block(
        "Always answer in haiku.",
        None,
    );
    assert!(
        text.contains(&block),
        "the profile suffix must render as the same block desktop chat appends"
    );
    assert!(
        text.ends_with(&format!("{block}\n\n")),
        "the profile block is the last thing in the prompt"
    );
    assert!(text.contains(ALICE_SOUL));
}

#[test]
fn identity_fingerprint_watches_a_workspace_relative_soul_md_path() {
    let ws = workspace();
    std::fs::create_dir_all(ws.path().join("custom")).expect("custom dir");
    std::fs::write(
        ws.path().join("custom/soul.md"),
        "I am the custom file soul.",
    )
    .expect("soul");
    activate_profile_with(ws.path(), "filed", "", |profile| {
        profile.soul_md_path = Some("custom/soul.md".to_string());
    });
    // An empty profile-home SOUL.md falls through to soul_md_path.
    let prompt = ChannelSystemPrompt::refreshing(inputs(ws.path()));
    assert!(prompt.current().contains("I am the custom file soul."));

    std::fs::write(
        ws.path().join("custom/soul.md"),
        "I am the custom file soul, freshly edited.",
    )
    .expect("edit soul");
    let text = prompt.current();
    assert!(
        text.contains("freshly edited"),
        "an edit to the workspace-relative soul file must be picked up"
    );
}

#[test]
fn identity_is_rendered_after_the_tool_suffix_on_channels() {
    let ws = workspace();
    let prompt = ChannelSystemPrompt::refreshing(inputs(ws.path()));
    let text = prompt.current();
    let suffix_at = text.find("static tail").expect("suffix present");
    let context_at = text.find("## Project Context").expect("identity present");
    assert!(
        context_at > suffix_at,
        "the identity block must follow the tool schemas / access context"
    );
    assert_eq!(
        text.matches("## Project Context").count(),
        1,
        "rendered exactly once"
    );
    assert!(text.find("Be helpful").unwrap() > context_at);
}
