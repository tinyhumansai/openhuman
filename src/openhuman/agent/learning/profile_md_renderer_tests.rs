use super::*;
use crate::openhuman::integrations::composio::profile_md::{block_end, block_start};
use std::sync::Arc;
use tempfile::TempDir;
use tinymemory_api::provider::{FacetState, FacetType, ProfileFacet, UserState};

fn make_cache() -> Arc<FacetCache> {
    Arc::new(crate::openhuman::agent::learning::test_profile::in_memory_cache())
}

async fn insert_facet(
    cache: &FacetCache,
    key: &str,
    value: &str,
    state: FacetState,
    user_state: UserState,
    stability: f64,
) {
    let facet = ProfileFacet {
        facet_id: format!("f-{key}"),
        facet_type: FacetType::Preference,
        key: key.into(),
        value: value.into(),
        confidence: 0.9,
        evidence_count: 3,
        source_segment_ids: None,
        first_seen_at: 1000.0,
        last_seen_at: 2000.0,
        state,
        stability,
        user_state,
        evidence_refs: vec![],
        class: key.split('/').next().map(|s| s.to_string()),
        cue_families: None,
    };
    cache.upsert(&facet).await.unwrap();
}

fn make_renderer() -> (Arc<FacetCache>, ProfileMdRenderer, TempDir) {
    let tmp = TempDir::new().unwrap();
    let cache = make_cache();
    let renderer = ProfileMdRenderer::new(Arc::clone(&cache), tmp.path().to_path_buf());
    (cache, renderer, tmp)
}

#[tokio::test]
async fn renders_active_facets_to_class_blocks() {
    let (cache, renderer, tmp) = make_renderer();
    insert_facet(
        &cache,
        "style/verbosity",
        "terse",
        FacetState::Active,
        UserState::Auto,
        2.0,
    )
    .await;
    insert_facet(
        &cache,
        "identity/name",
        "Alice",
        FacetState::Active,
        UserState::Auto,
        1.8,
    )
    .await;
    insert_facet(
        &cache,
        "tooling/editor",
        "neovim",
        FacetState::Active,
        UserState::Auto,
        1.5,
    )
    .await;
    insert_facet(
        &cache,
        "veto/no-em-dashes",
        "avoid em dashes in prose",
        FacetState::Active,
        UserState::Auto,
        1.2,
    )
    .await;
    insert_facet(
        &cache,
        "goal/learn-rust",
        "Learn Rust this year",
        FacetState::Active,
        UserState::Auto,
        1.0,
    )
    .await;

    renderer.render().await.unwrap();

    let body = std::fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    assert!(
        body.contains("- **verbosity**: terse"),
        "style block:\n{body}"
    );
    assert!(
        body.contains("- **name**: Alice"),
        "identity block:\n{body}"
    );
    assert!(
        body.contains("- **editor**: neovim"),
        "tooling block:\n{body}"
    );
    assert!(
        body.contains("- **no-em-dashes**: avoid em dashes"),
        "vetoes block:\n{body}"
    );
    assert!(
        body.contains("- Learn Rust this year"),
        "goals block:\n{body}"
    );
}

#[tokio::test]
async fn skips_empty_classes_renders_placeholder() {
    let (cache, renderer, tmp) = make_renderer();
    // Only insert a style facet; all other classes will be empty.
    insert_facet(
        &cache,
        "style/verbosity",
        "terse",
        FacetState::Active,
        UserState::Auto,
        2.0,
    )
    .await;

    renderer.render().await.unwrap();

    let body = std::fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    // Empty classes get the placeholder.
    assert!(
        body.contains("*(no entries yet)*"),
        "placeholder missing:\n{body}"
    );
    // The style class has real content.
    assert!(body.contains("- **verbosity**: terse"));
}

#[tokio::test]
async fn pinned_facets_marked_in_output() {
    let (cache, renderer, tmp) = make_renderer();
    insert_facet(
        &cache,
        "style/format",
        "markdown",
        FacetState::Active,
        UserState::Pinned,
        1.0,
    )
    .await;

    renderer.render().await.unwrap();

    let body = std::fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    assert!(
        body.contains("*(pinned)*"),
        "pinned marker missing:\n{body}"
    );
    assert!(body.contains("- **format**: markdown *(pinned)*"));
}

#[tokio::test]
async fn provisional_facets_excluded_from_output() {
    let (cache, renderer, tmp) = make_renderer();
    insert_facet(
        &cache,
        "style/tone",
        "formal",
        FacetState::Provisional,
        UserState::Auto,
        0.8,
    )
    .await;
    insert_facet(
        &cache,
        "style/verbosity",
        "terse",
        FacetState::Active,
        UserState::Auto,
        2.0,
    )
    .await;

    renderer.render().await.unwrap();

    let body = std::fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    assert!(
        !body.contains("formal"),
        "provisional must not appear:\n{body}"
    );
    assert!(body.contains("terse"));
}

#[tokio::test]
async fn re_renders_idempotently_on_repeated_cache_rebuilt() {
    let (cache, renderer, tmp) = make_renderer();
    insert_facet(
        &cache,
        "style/verbosity",
        "terse",
        FacetState::Active,
        UserState::Auto,
        2.0,
    )
    .await;

    renderer.render().await.unwrap();
    let body1 = std::fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
    renderer.render().await.unwrap();
    let body2 = std::fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();

    assert_eq!(body1, body2, "second render should be idempotent");
}

#[tokio::test]
async fn renders_dont_clobber_connected_accounts_block() {
    let (cache, renderer, tmp) = make_renderer();
    // Manually write a connected-accounts block first.
    let ca_content = format!(
        "{}\n## Connected Accounts\n\n- <!-- acct:gmail:c-1 --> **Gmail** (c-1): jane@test.com\n{}\n",
        block_start("connected-accounts"),
        block_end("connected-accounts"),
    );
    let profile_path = tmp.path().join("PROFILE.md");
    std::fs::write(&profile_path, format!("# User Profile\n\n{ca_content}")).unwrap();

    insert_facet(
        &cache,
        "style/verbosity",
        "terse",
        FacetState::Active,
        UserState::Auto,
        2.0,
    )
    .await;
    renderer.render().await.unwrap();

    let body = std::fs::read_to_string(&profile_path).unwrap();
    // connected-accounts block preserved.
    assert!(
        body.contains("acct:gmail:c-1"),
        "CA block clobbered:\n{body}"
    );
    assert!(
        body.contains("jane@test.com"),
        "CA block clobbered:\n{body}"
    );
    // Style block also written.
    assert!(body.contains("terse"));
}

#[tokio::test]
async fn renders_dont_touch_user_authored_text_outside_blocks() {
    let (cache, renderer, tmp) = make_renderer();
    let profile_path = tmp.path().join("PROFILE.md");
    std::fs::write(
        &profile_path,
        "# User Profile\n\nHand-written note by the user.\n",
    )
    .unwrap();

    insert_facet(
        &cache,
        "style/verbosity",
        "terse",
        FacetState::Active,
        UserState::Auto,
        2.0,
    )
    .await;
    renderer.render().await.unwrap();

    let body = std::fs::read_to_string(&profile_path).unwrap();
    assert!(
        body.contains("Hand-written note by the user."),
        "user text lost:\n{body}"
    );
    assert!(body.contains("terse"));
}

#[test]
fn subscribes_and_handles_cache_rebuilt_event() {
    // Verify that ProfileMdRenderer::subscribe compiles and returns a handle.
    // Full async event delivery is tested in the integration test.
    let tmp = TempDir::new().unwrap();
    let cache = make_cache();
    let renderer = Arc::new(ProfileMdRenderer::new(cache, tmp.path().to_path_buf()));
    // subscribe_global requires a running runtime; just verify the type works.
    let _renderer_ref = Arc::clone(&renderer);
    // We can't call subscribe_global in a unit test without a tokio runtime,
    // but we verify the subscriber type implements EventHandler correctly.
    let subscriber = RendererSubscriber(renderer);
    assert_eq!(subscriber.name(), "learning::profile_md_renderer");
    assert_eq!(subscriber.domains(), Some(["memory"].as_slice()));
}
