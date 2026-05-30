//! E2E integration tests for multi-agent personalities.
//!
//! Covers the full personality feature surface end-to-end without requiring
//! the JSON-RPC HTTP stack:
//!
//! 1. Profile lifecycle — backwards compat, suffix auto-assignment, field roundtrip.
//! 2. Memory isolation — two personalities have separate SQLite stores.
//! 3. Personality file resolution — soul/memory.md fallback chain.
//! 4. Prompt section behaviour — IdentitySection / UserFilesSection / PersonalityRosterSection.
//! 5. Integration filtering — allowlist / passthrough / empty.
//! 6. Thread-personality binding — persists through ConversationStore.
//!
//! Run with: `cargo test --test personality_e2e`

use std::collections::HashSet;
use std::sync::Arc;

use serde_json::json;
use tempfile::tempdir;

use openhuman_core::openhuman::agent::personality_paths::{
    filter_integrations, memory_subdir_for_suffix, memory_tree_subdir_for_suffix,
    resolve_personality_memory_md, resolve_personality_soul, session_raw_subdir_for_suffix,
    HasToolkit, PersonalityContext,
};
use openhuman_core::openhuman::agent::profiles::{
    built_in_profiles, AgentProfile, AgentProfileStore, DEFAULT_PROFILE_ID,
};
use openhuman_core::openhuman::agent::prompts::types::LearnedContextData;
use openhuman_core::openhuman::agent::prompts::{
    IdentitySection, PersonalityRosterEntry, PersonalityRosterSection, PromptContext,
    PromptSection, ToolCallFormat, UserFilesSection,
};
use openhuman_core::openhuman::embeddings::NoopEmbedding;
use openhuman_core::openhuman::memory::{NamespaceDocumentInput, UnifiedMemory};
use openhuman_core::openhuman::memory_conversations::{
    ensure_thread, list_threads, update_thread_title, ConversationStore, CreateConversationThread,
};

// ─────────────────────────────────────────────────────────────────────────────
// Test helpers
// ─────────────────────────────────────────────────────────────────────────────

fn make_profile(id: &str, name: &str) -> AgentProfile {
    AgentProfile {
        id: id.to_string(),
        name: name.to_string(),
        description: format!("{name} personality"),
        agent_id: "orchestrator".to_string(),
        model_override: None,
        temperature: None,
        system_prompt_suffix: None,
        allowed_tools: None,
        built_in: false,
        avatar_url: None,
        voice_id: None,
        soul_md: None,
        soul_md_path: None,
        composio_integrations: None,
        memory_dir_suffix: None,
        is_master: false,
        sort_order: None,
    }
}

fn empty_prompt_context<'a>(workspace_dir: &'a std::path::Path) -> PromptContext<'a> {
    let visible_tool_names: &'a HashSet<String> = Box::leak(Box::new(HashSet::new()));
    PromptContext {
        workspace_dir,
        model_name: "test-model",
        agent_id: "orchestrator",
        tools: &[],
        skills: &[],
        workflows: &[],
        dispatcher_instructions: "",
        learned: LearnedContextData::default(),
        visible_tool_names,
        tool_call_format: ToolCallFormat::PFormat,
        connected_integrations: &[],
        connected_identities_md: String::new(),
        include_profile: false,
        include_memory_md: false,
        curated_snapshot: None,
        user_identity: None,
        personality_soul_md: None,
        personality_memory_md: None,
        personality_roster: vec![],
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// 1. Profile lifecycle
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn backwards_compat_old_profiles_json_deserializes_cleanly() {
    let tmp = tempdir().expect("tempdir");
    let json = r#"{
        "activeProfileId": "default",
        "profiles": [{
            "id": "default",
            "name": "Default",
            "description": "Old profile pre-personalities.",
            "agentId": "orchestrator",
            "builtIn": true
        }, {
            "id": "research",
            "name": "Research",
            "description": "",
            "agentId": "researcher",
            "builtIn": true
        }]
    }"#;
    std::fs::write(tmp.path().join("agent_profiles.json"), json).expect("write");

    let store = AgentProfileStore::new(tmp.path().to_path_buf());
    let state = store.load().expect("load");

    let default = state
        .profiles
        .iter()
        .find(|p| p.id == DEFAULT_PROFILE_ID)
        .expect("default");
    assert!(default.is_master, "default should be master on first read");
    assert_eq!(default.memory_dir_suffix.as_deref(), Some(""));
    assert_eq!(default.avatar_url, None);
}

#[test]
fn three_personalities_get_sequential_memory_suffixes() {
    let tmp = tempdir().expect("tempdir");
    let store = AgentProfileStore::new(tmp.path().to_path_buf());

    let state = store.upsert(make_profile("alice", "Alice")).expect("alice");
    let alice = state.profiles.iter().find(|p| p.id == "alice").unwrap();
    assert_eq!(alice.memory_dir_suffix.as_deref(), Some("-1"));

    let state = store.upsert(make_profile("bob", "Bob")).expect("bob");
    let bob = state.profiles.iter().find(|p| p.id == "bob").unwrap();
    assert_eq!(bob.memory_dir_suffix.as_deref(), Some("-2"));

    let state = store.upsert(make_profile("carol", "Carol")).expect("carol");
    let carol = state.profiles.iter().find(|p| p.id == "carol").unwrap();
    assert_eq!(carol.memory_dir_suffix.as_deref(), Some("-3"));
}

#[test]
fn deleted_suffix_is_reused_by_next_personality() {
    let tmp = tempdir().expect("tempdir");
    let store = AgentProfileStore::new(tmp.path().to_path_buf());

    store.upsert(make_profile("alice", "Alice")).expect("alice");
    store.upsert(make_profile("bob", "Bob")).expect("bob");
    store.upsert(make_profile("carol", "Carol")).expect("carol");

    store.delete("bob").expect("delete bob");

    let state = store.upsert(make_profile("dave", "Dave")).expect("dave");
    let dave = state.profiles.iter().find(|p| p.id == "dave").unwrap();
    assert_eq!(
        dave.memory_dir_suffix.as_deref(),
        Some("-2"),
        "freed suffix -2 should be reused"
    );
}

#[test]
fn personality_fields_roundtrip_through_upsert() {
    let tmp = tempdir().expect("tempdir");
    let store = AgentProfileStore::new(tmp.path().to_path_buf());

    let mut alice = make_profile("alice", "Alice");
    alice.avatar_url = Some("https://example.com/alice.png".to_string());
    alice.voice_id = Some("voice-alice-123".to_string());
    alice.soul_md = Some("I am Alice.".to_string());
    alice.composio_integrations = Some(vec!["slack".to_string(), "gmail".to_string()]);
    alice.sort_order = Some(5);

    store.upsert(alice).expect("upsert");

    let state = store.load().expect("reload");
    let alice = state.profiles.iter().find(|p| p.id == "alice").unwrap();
    assert_eq!(
        alice.avatar_url.as_deref(),
        Some("https://example.com/alice.png")
    );
    assert_eq!(alice.voice_id.as_deref(), Some("voice-alice-123"));
    assert_eq!(alice.soul_md.as_deref(), Some("I am Alice."));
    assert_eq!(
        alice.composio_integrations.as_ref().unwrap(),
        &["slack", "gmail"]
    );
    assert_eq!(alice.sort_order, Some(5));
}

#[test]
fn default_profile_memory_suffix_cannot_be_overridden() {
    let tmp = tempdir().expect("tempdir");
    let store = AgentProfileStore::new(tmp.path().to_path_buf());

    let mut default_override = built_in_profiles()
        .into_iter()
        .find(|p| p.id == DEFAULT_PROFILE_ID)
        .expect("default profile in built-ins");
    default_override.memory_dir_suffix = Some("-99".to_string());
    default_override.avatar_url = Some("https://example.com/default.png".to_string());

    let state = store.upsert(default_override).expect("upsert");
    let default = state
        .profiles
        .iter()
        .find(|p| p.id == DEFAULT_PROFILE_ID)
        .unwrap();
    assert_eq!(
        default.memory_dir_suffix.as_deref(),
        Some(""),
        "default suffix must stay empty regardless of user input"
    );
    assert_eq!(
        default.avatar_url.as_deref(),
        Some("https://example.com/default.png")
    );
    assert!(default.is_master);
}

// ─────────────────────────────────────────────────────────────────────────────
// 2. Memory isolation
// ─────────────────────────────────────────────────────────────────────────────

#[tokio::test]
async fn two_personalities_have_isolated_sqlite_stores() {
    let tmp = tempdir().expect("tempdir");

    let mem_default = UnifiedMemory::new_with_memory_dir(
        tmp.path(),
        &memory_subdir_for_suffix(""),
        Arc::new(NoopEmbedding),
        None,
    )
    .expect("default memory");
    let mem_alice = UnifiedMemory::new_with_memory_dir(
        tmp.path(),
        &memory_subdir_for_suffix("-1"),
        Arc::new(NoopEmbedding),
        None,
    )
    .expect("alice memory");

    assert_ne!(mem_default.db_path(), mem_alice.db_path());
    assert!(mem_default.db_path().ends_with("memory/memory.db"));
    assert!(mem_alice.db_path().ends_with("memory-1/memory.db"));
    assert!(mem_default.db_path().exists());
    assert!(mem_alice.db_path().exists());

    mem_default
        .upsert_document(NamespaceDocumentInput {
            namespace: "shared".to_string(),
            key: "default-only".to_string(),
            title: "Default's note".to_string(),
            content: "Only the default agent knows about this.".to_string(),
            source_type: "doc".to_string(),
            priority: "high".to_string(),
            tags: vec![],
            metadata: json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        })
        .await
        .expect("write default");

    mem_alice
        .upsert_document(NamespaceDocumentInput {
            namespace: "shared".to_string(),
            key: "alice-only".to_string(),
            title: "Alice's note".to_string(),
            content: "Only Alice knows about this.".to_string(),
            source_type: "doc".to_string(),
            priority: "high".to_string(),
            tags: vec![],
            metadata: json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        })
        .await
        .expect("write alice");

    let default_hits = mem_default
        .query_namespace_ranked("shared", "note", 10)
        .await
        .expect("default query");
    let alice_hits = mem_alice
        .query_namespace_ranked("shared", "note", 10)
        .await
        .expect("alice query");

    let default_keys: Vec<_> = default_hits.iter().map(|h| h.key.as_str()).collect();
    let alice_keys: Vec<_> = alice_hits.iter().map(|h| h.key.as_str()).collect();

    assert!(
        default_keys.contains(&"default-only"),
        "default sees its own doc"
    );
    assert!(
        !default_keys.contains(&"alice-only"),
        "default must NOT see alice's doc"
    );
    assert!(alice_keys.contains(&"alice-only"), "alice sees her own doc");
    assert!(
        !alice_keys.contains(&"default-only"),
        "alice must NOT see default's doc"
    );
}

#[tokio::test]
async fn personality_memory_persists_across_reopens() {
    let tmp = tempdir().expect("tempdir");

    {
        let mem = UnifiedMemory::new_with_memory_dir(
            tmp.path(),
            &memory_subdir_for_suffix("-1"),
            Arc::new(NoopEmbedding),
            None,
        )
        .expect("open 1");
        mem.upsert_document(NamespaceDocumentInput {
            namespace: "alice".to_string(),
            key: "persistent".to_string(),
            title: "Persistent note".to_string(),
            content: "This must survive a reopen.".to_string(),
            source_type: "doc".to_string(),
            priority: "high".to_string(),
            tags: vec![],
            metadata: json!({}),
            category: "core".to_string(),
            session_id: None,
            document_id: None,
        })
        .await
        .expect("write");
    }

    let mem2 = UnifiedMemory::new_with_memory_dir(
        tmp.path(),
        &memory_subdir_for_suffix("-1"),
        Arc::new(NoopEmbedding),
        None,
    )
    .expect("reopen");
    let hits = mem2
        .query_namespace_ranked("alice", "persistent", 10)
        .await
        .expect("query");
    assert!(hits.iter().any(|h| h.key == "persistent"));
}

// ─────────────────────────────────────────────────────────────────────────────
// 3. Personality file resolution
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn resolve_soul_falls_back_through_path_inline_none() {
    let tmp = tempdir().expect("tempdir");
    let mut profile = make_profile("alice", "Alice");

    // No overrides → None.
    assert!(resolve_personality_soul(tmp.path(), &profile).is_none());

    // Inline only.
    profile.soul_md = Some("Inline soul.".to_string());
    assert_eq!(
        resolve_personality_soul(tmp.path(), &profile).as_deref(),
        Some("Inline soul.")
    );

    // File path takes precedence over inline.
    let soul_path = tmp.path().join("souls").join("alice.md");
    std::fs::create_dir_all(soul_path.parent().unwrap()).unwrap();
    std::fs::write(&soul_path, "File-based soul.").unwrap();
    profile.soul_md_path = Some("souls/alice.md".to_string());
    assert_eq!(
        resolve_personality_soul(tmp.path(), &profile).as_deref(),
        Some("File-based soul.")
    );

    // Missing file falls back to inline.
    profile.soul_md_path = Some("souls/missing.md".to_string());
    assert_eq!(
        resolve_personality_soul(tmp.path(), &profile).as_deref(),
        Some("Inline soul.")
    );
}

#[test]
fn resolve_memory_md_reads_personality_dir() {
    let tmp = tempdir().expect("tempdir");
    let profile = make_profile("alice", "Alice");

    assert!(resolve_personality_memory_md(tmp.path(), &profile).is_none());

    let mem_path = tmp
        .path()
        .join("personalities")
        .join("alice")
        .join("MEMORY.md");
    std::fs::create_dir_all(mem_path.parent().unwrap()).unwrap();
    std::fs::write(&mem_path, "Alice's curated memory.").unwrap();

    assert_eq!(
        resolve_personality_memory_md(tmp.path(), &profile).as_deref(),
        Some("Alice's curated memory.")
    );
}

#[test]
fn personality_context_aggregates_all_overrides() {
    let tmp = tempdir().expect("tempdir");
    let mut alice = make_profile("alice", "Alice");
    alice.memory_dir_suffix = Some("-1".to_string());
    alice.voice_id = Some("voice-alice".to_string());
    alice.soul_md = Some("I am Alice.".to_string());
    alice.composio_integrations = Some(vec!["slack".to_string()]);

    let mem_path = tmp
        .path()
        .join("personalities")
        .join("alice")
        .join("MEMORY.md");
    std::fs::create_dir_all(mem_path.parent().unwrap()).unwrap();
    std::fs::write(&mem_path, "Curated.").unwrap();

    let ctx = PersonalityContext::from_profile(tmp.path(), alice);

    assert_eq!(ctx.memory_suffix, "-1");
    assert_eq!(ctx.voice_id.as_deref(), Some("voice-alice"));
    assert_eq!(ctx.soul_md_override.as_deref(), Some("I am Alice."));
    assert_eq!(ctx.memory_md_override.as_deref(), Some("Curated."));
    assert_eq!(ctx.composio_allowlist.as_ref().unwrap(), &["slack"]);
}

#[test]
fn subdir_helpers_handle_empty_and_numeric_suffixes() {
    assert_eq!(memory_subdir_for_suffix(""), "memory");
    assert_eq!(memory_subdir_for_suffix("-1"), "memory-1");
    assert_eq!(memory_tree_subdir_for_suffix(""), "memory_tree");
    assert_eq!(memory_tree_subdir_for_suffix("-2"), "memory_tree-2");
    assert_eq!(session_raw_subdir_for_suffix(""), "session_raw");
    assert_eq!(session_raw_subdir_for_suffix("-3"), "session_raw-3");
}

// ─────────────────────────────────────────────────────────────────────────────
// 4. Prompt section behaviour
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn identity_section_uses_personality_soul_when_set() {
    let tmp = tempdir().expect("tempdir");
    let mut ctx = empty_prompt_context(tmp.path());
    ctx.personality_soul_md = Some("# I am Alice\n\nI specialise in research.".to_string());

    let rendered = IdentitySection.build(&ctx).expect("render");
    assert!(
        rendered.contains("I am Alice"),
        "personality soul must be in rendered prompt: {rendered}"
    );
    assert!(rendered.contains("specialise in research"));
}

#[test]
fn user_files_section_prefers_personality_memory_md() {
    let tmp = tempdir().expect("tempdir");

    // Write a workspace-level MEMORY.md so we can prove personality override wins.
    std::fs::write(
        tmp.path().join("MEMORY.md"),
        "WORKSPACE memory contents — should not appear.",
    )
    .expect("write workspace memory");

    let mut ctx = empty_prompt_context(tmp.path());
    ctx.include_memory_md = true;
    ctx.personality_memory_md = Some("PERSONALITY memory contents.".to_string());

    let rendered = UserFilesSection.build(&ctx).expect("render");
    assert!(
        rendered.contains("PERSONALITY memory"),
        "personality memory should be rendered: {rendered}"
    );
    assert!(
        !rendered.contains("WORKSPACE memory"),
        "workspace memory must not be rendered when personality memory is set"
    );
}

#[test]
fn user_files_section_falls_back_to_workspace_memory_md() {
    let tmp = tempdir().expect("tempdir");
    std::fs::write(tmp.path().join("MEMORY.md"), "Workspace fallback memory.").expect("write");

    let mut ctx = empty_prompt_context(tmp.path());
    ctx.include_memory_md = true;
    // No personality override.

    let rendered = UserFilesSection.build(&ctx).expect("render");
    assert!(
        rendered.contains("Workspace fallback memory"),
        "should fall back to workspace MEMORY.md: {rendered}"
    );
}

#[test]
fn personality_roster_renders_entries_with_truncated_summaries() {
    let tmp = tempdir().expect("tempdir");
    let mut ctx = empty_prompt_context(tmp.path());

    let long_summary = "A".repeat(300);
    ctx.personality_roster = vec![
        PersonalityRosterEntry {
            id: "alice".to_string(),
            name: "Alice".to_string(),
            description: "Research specialist.".to_string(),
            memory_summary: Some(long_summary.clone()),
        },
        PersonalityRosterEntry {
            id: "bob".to_string(),
            name: "Bob".to_string(),
            description: "Code reviewer.".to_string(),
            memory_summary: None,
        },
    ];

    let rendered = PersonalityRosterSection.build(&ctx).expect("render");
    assert!(rendered.contains("Available Personalities"));
    assert!(rendered.contains("Alice"));
    assert!(rendered.contains("alice"));
    assert!(rendered.contains("Research specialist."));
    assert!(rendered.contains("Bob"));
    assert!(rendered.contains("Code reviewer."));
    // Summary capped at 200 chars + ellipsis.
    assert!(
        !rendered.contains(&"A".repeat(250)),
        "summary should be truncated below 250 chars"
    );
    assert!(rendered.contains("…"));
}

#[test]
fn personality_roster_renders_empty_when_no_entries() {
    let tmp = tempdir().expect("tempdir");
    let ctx = empty_prompt_context(tmp.path());
    let rendered = PersonalityRosterSection.build(&ctx).expect("render");
    assert!(rendered.is_empty(), "empty roster should render nothing");
}

// ─────────────────────────────────────────────────────────────────────────────
// 5. Integration filtering
// ─────────────────────────────────────────────────────────────────────────────

#[derive(Clone, Debug)]
struct FakeIntegration {
    toolkit: String,
}

impl HasToolkit for FakeIntegration {
    fn toolkit_name(&self) -> &str {
        &self.toolkit
    }
}

#[test]
fn filter_integrations_none_is_passthrough() {
    let all = vec![
        FakeIntegration {
            toolkit: "slack".into(),
        },
        FakeIntegration {
            toolkit: "gmail".into(),
        },
        FakeIntegration {
            toolkit: "notion".into(),
        },
    ];
    let filtered = filter_integrations(&all, None);
    assert_eq!(filtered.len(), 3);
}

#[test]
fn filter_integrations_allowlist_is_case_insensitive() {
    let all = vec![
        FakeIntegration {
            toolkit: "slack".into(),
        },
        FakeIntegration {
            toolkit: "gmail".into(),
        },
        FakeIntegration {
            toolkit: "notion".into(),
        },
    ];
    let allowed = vec!["SLACK".to_string(), "Gmail".to_string()];
    let filtered = filter_integrations(&all, Some(&allowed));
    let names: Vec<_> = filtered.iter().map(|i| i.toolkit.as_str()).collect();
    assert_eq!(names, vec!["slack", "gmail"]);
}

#[test]
fn filter_integrations_empty_allowlist_returns_nothing() {
    let all = vec![FakeIntegration {
        toolkit: "slack".into(),
    }];
    let empty: Vec<String> = vec![];
    let filtered = filter_integrations(&all, Some(&empty));
    assert!(filtered.is_empty());
}

// ─────────────────────────────────────────────────────────────────────────────
// 6. Thread-personality binding
// ─────────────────────────────────────────────────────────────────────────────

#[test]
fn thread_personality_id_persists_through_store() {
    let tmp = tempdir().expect("tempdir");
    let store = ConversationStore::new(tmp.path().to_path_buf());

    let thread = store
        .ensure_thread(CreateConversationThread {
            id: "thread-alice".to_string(),
            title: "Alice chat".to_string(),
            created_at: "2026-05-27T10:00:00Z".to_string(),
            parent_thread_id: None,
            labels: Some(vec!["work".to_string()]),
            personality_id: Some("alice".to_string()),
        })
        .expect("ensure");

    assert_eq!(thread.personality_id.as_deref(), Some("alice"));

    let listed = store.list_threads().expect("list");
    let alice_thread = listed.iter().find(|t| t.id == "thread-alice").unwrap();
    assert_eq!(alice_thread.personality_id.as_deref(), Some("alice"));
}

#[test]
fn threads_without_personality_id_remain_none() {
    let tmp = tempdir().expect("tempdir");
    let store = ConversationStore::new(tmp.path().to_path_buf());

    store
        .ensure_thread(CreateConversationThread {
            id: "legacy-thread".to_string(),
            title: "Legacy".to_string(),
            created_at: "2026-05-27T10:00:00Z".to_string(),
            parent_thread_id: None,
            labels: None,
            personality_id: None,
        })
        .expect("ensure");

    let listed = store.list_threads().expect("list");
    let legacy = listed.iter().find(|t| t.id == "legacy-thread").unwrap();
    assert!(legacy.personality_id.is_none());
}

#[test]
fn updating_thread_title_preserves_personality_id() {
    let tmp = tempdir().expect("tempdir");
    let store = ConversationStore::new(tmp.path().to_path_buf());

    store
        .ensure_thread(CreateConversationThread {
            id: "thread-bob".to_string(),
            title: "Original".to_string(),
            created_at: "2026-05-27T10:00:00Z".to_string(),
            parent_thread_id: None,
            labels: None,
            personality_id: Some("bob".to_string()),
        })
        .expect("ensure");

    store
        .update_thread_title("thread-bob", "Updated title", "2026-05-27T11:00:00Z")
        .expect("update title");

    let listed = store.list_threads().expect("list");
    let bob = listed.iter().find(|t| t.id == "thread-bob").unwrap();
    assert_eq!(bob.title, "Updated title");
    assert_eq!(
        bob.personality_id.as_deref(),
        Some("bob"),
        "personality_id must survive title updates"
    );
}

#[test]
fn module_level_helpers_round_trip_personality_id() {
    // Verify the free-function API (used by some callers) also propagates personality_id.
    let tmp = tempdir().expect("tempdir");
    let workspace = tmp.path().to_path_buf();

    let thread = ensure_thread(
        workspace.clone(),
        CreateConversationThread {
            id: "module-thread".to_string(),
            title: "via module".to_string(),
            created_at: "2026-05-27T12:00:00Z".to_string(),
            parent_thread_id: None,
            labels: None,
            personality_id: Some("carol".to_string()),
        },
    )
    .expect("ensure");
    assert_eq!(thread.personality_id.as_deref(), Some("carol"));

    update_thread_title(
        workspace.clone(),
        "module-thread",
        "Updated",
        "2026-05-27T13:00:00Z",
    )
    .expect("update title");

    let threads = list_threads(workspace).expect("list");
    let found = threads.iter().find(|t| t.id == "module-thread").unwrap();
    assert_eq!(found.personality_id.as_deref(), Some("carol"));
    assert_eq!(found.title, "Updated");
}
