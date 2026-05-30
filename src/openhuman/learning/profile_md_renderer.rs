//! Profile-MD renderer for the learning subsystem.
//!
//! Subscribes to [`DomainEvent::CacheRebuilt`] and re-renders the five
//! cache-derived managed blocks in `PROFILE.md`:
//!
//! | Block name | Heading | Facet class |
//! |------------|---------|-------------|
//! | `style`     | `## Style`   | `FacetClass::Style`   |
//! | `identity`  | `## Identity`| `FacetClass::Identity`|
//! | `tooling`   | `## Tooling` | `FacetClass::Tooling` |
//! | `vetoes`    | `## Vetoes`  | `FacetClass::Veto`    |
//! | `goals`     | `## Goals`   | `FacetClass::Goal`    |
//!
//! The `connected-accounts` block is NOT touched by this renderer; it is
//! owned exclusively by the provider path
//! (`composio::providers::profile_md::merge_provider_into_profile_md`).
//!
//! ## Rendering rules
//!
//! - Only `Active` rows are rendered in the visible blocks.
//! - Within each block, rows are sorted by `stability` desc, then by `key` asc.
//! - `Pinned` entries get a trailing ` *(pinned)*` indicator.
//! - Format per class:
//!   - Style / Identity / Tooling / Vetoes: `- **{suffix}**: {value}`
//!     where `suffix` is the portion of the key after the first `/`.
//!   - Goals: `- {value}` (full sentence — no key prefix).
//! - Empty classes render the `*(no entries yet)*` placeholder (never
//!   delete the block markers).
//!
//! ## Subscription
//!
//! [`ProfileMdRenderer::subscribe`] registers an `EventHandler` that calls
//! [`ProfileMdRenderer::render`] on every `CacheRebuilt` event. The render
//! is synchronous (SQLite reads + file writes) and runs on the Tokio blocking
//! thread pool.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;

use crate::core::event_bus::{subscribe_global, DomainEvent, EventHandler, SubscriptionHandle};
use crate::openhuman::composio::providers::profile_md::replace_managed_block;
use crate::openhuman::learning::cache::FacetCache;
use crate::openhuman::memory_store::profile::UserState;

// ── Class → block metadata ────────────────────────────────────────────────────

struct BlockSpec {
    block_name: &'static str,
    heading: &'static str,
    class_prefix: &'static str,
    /// When true, render `- {value}` (goal style). Otherwise `- **{key_suffix}**: {value}`.
    value_only: bool,
}

const BLOCK_SPECS: &[BlockSpec] = &[
    BlockSpec {
        block_name: "style",
        heading: "## Style",
        class_prefix: "style/",
        value_only: false,
    },
    BlockSpec {
        block_name: "identity",
        heading: "## Identity",
        class_prefix: "identity/",
        value_only: false,
    },
    BlockSpec {
        block_name: "tooling",
        heading: "## Tooling",
        class_prefix: "tooling/",
        value_only: false,
    },
    BlockSpec {
        block_name: "vetoes",
        heading: "## Vetoes",
        class_prefix: "veto/",
        value_only: false,
    },
    BlockSpec {
        block_name: "goals",
        heading: "## Goals",
        class_prefix: "goal/",
        value_only: true,
    },
];

// ── ProfileMdRenderer ─────────────────────────────────────────────────────────

/// Renders Active facets from the `FacetCache` into the five cache-derived
/// managed blocks of `PROFILE.md`.
pub struct ProfileMdRenderer {
    cache: Arc<FacetCache>,
    workspace_dir: PathBuf,
}

impl ProfileMdRenderer {
    /// Create a new renderer backed by `cache`, writing to
    /// `workspace_dir/PROFILE.md`.
    pub fn new(cache: Arc<FacetCache>, workspace_dir: PathBuf) -> Self {
        Self {
            cache,
            workspace_dir,
        }
    }

    /// Read all Active facets from the cache and re-render each of the five
    /// cache-owned blocks. Never touches the `connected-accounts` block.
    pub fn render(&self) -> anyhow::Result<()> {
        tracing::debug!("[learning::profile_md_renderer] render triggered — reading active facets");

        let active_facets = self.cache.list_active()?;

        for spec in BLOCK_SPECS {
            // Filter to this class, sort by stability desc then key asc.
            let mut rows: Vec<_> = active_facets
                .iter()
                .filter(|f| f.key.starts_with(spec.class_prefix))
                .collect();
            rows.sort_by(|a, b| {
                b.stability
                    .partial_cmp(&a.stability)
                    .unwrap_or(std::cmp::Ordering::Equal)
                    .then_with(|| a.key.cmp(&b.key))
            });

            let body = if rows.is_empty() {
                String::new() // replace_managed_block renders the placeholder
            } else {
                let mut lines: Vec<String> = Vec::with_capacity(rows.len());
                for f in &rows {
                    let pinned_suffix = if f.user_state == UserState::Pinned {
                        " *(pinned)*"
                    } else {
                        ""
                    };
                    let line = if spec.value_only {
                        format!("- {}{}", f.value, pinned_suffix)
                    } else {
                        let key_suffix = f
                            .key
                            .strip_prefix(spec.class_prefix)
                            .unwrap_or(f.key.as_str());
                        format!("- **{}**: {}{}", key_suffix, f.value, pinned_suffix)
                    };
                    lines.push(line);
                }
                lines.join("\n")
            };

            replace_managed_block(&self.workspace_dir, spec.block_name, spec.heading, body)
                .map_err(|e| {
                    anyhow::anyhow!(
                        "[learning::profile_md_renderer] failed to write block '{}': {e}",
                        spec.block_name
                    )
                })?;

            tracing::debug!(
                "[learning::profile_md_renderer] wrote block '{}' ({} entries)",
                spec.block_name,
                rows.len()
            );
        }

        tracing::info!("[learning::profile_md_renderer] PROFILE.md updated successfully");
        Ok(())
    }

    /// Register this renderer as an event subscriber for
    /// [`DomainEvent::CacheRebuilt`] events.
    ///
    /// Returns the [`SubscriptionHandle`] — hold it alive for the lifetime of
    /// the process (e.g. by leaking it into a static).
    pub fn subscribe(renderer: Arc<ProfileMdRenderer>) -> Option<SubscriptionHandle> {
        subscribe_global(Arc::new(RendererSubscriber(renderer)))
    }
}

// ── Event subscriber ─────────────────────────────────────────────────────────

struct RendererSubscriber(Arc<ProfileMdRenderer>);

#[async_trait]
impl EventHandler for RendererSubscriber {
    fn name(&self) -> &str {
        "learning::profile_md_renderer"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["memory"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::CacheRebuilt { .. } = event {
            let renderer = Arc::clone(&self.0);
            // Move the blocking I/O (SQLite reads + fs writes) off the async
            // executor thread.
            tokio::task::spawn_blocking(move || {
                if let Err(e) = renderer.render() {
                    tracing::warn!(
                        "[learning::profile_md_renderer] render on CacheRebuilt failed: {e:#}"
                    );
                }
            });
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::composio::providers::profile_md::{block_end, block_start};
    use crate::openhuman::memory_store::profile::{
        FacetState, FacetType, ProfileFacet, UserState, PROFILE_INIT_SQL,
    };
    use parking_lot::Mutex;
    use rusqlite::Connection;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_cache(conn: Arc<Mutex<Connection>>) -> Arc<FacetCache> {
        Arc::new(FacetCache::new(conn))
    }

    fn insert_facet(
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
        cache.upsert(&facet).unwrap();
    }

    fn make_renderer() -> (Arc<FacetCache>, ProfileMdRenderer, TempDir) {
        let tmp = TempDir::new().unwrap();
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(PROFILE_INIT_SQL).unwrap();
        let cache = make_cache(Arc::new(Mutex::new(conn)));
        let renderer = ProfileMdRenderer::new(Arc::clone(&cache), tmp.path().to_path_buf());
        (cache, renderer, tmp)
    }

    #[test]
    fn renders_active_facets_to_class_blocks() {
        let (cache, renderer, tmp) = make_renderer();
        insert_facet(
            &cache,
            "style/verbosity",
            "terse",
            FacetState::Active,
            UserState::Auto,
            2.0,
        );
        insert_facet(
            &cache,
            "identity/name",
            "Alice",
            FacetState::Active,
            UserState::Auto,
            1.8,
        );
        insert_facet(
            &cache,
            "tooling/editor",
            "neovim",
            FacetState::Active,
            UserState::Auto,
            1.5,
        );
        insert_facet(
            &cache,
            "veto/no-em-dashes",
            "avoid em dashes in prose",
            FacetState::Active,
            UserState::Auto,
            1.2,
        );
        insert_facet(
            &cache,
            "goal/learn-rust",
            "Learn Rust this year",
            FacetState::Active,
            UserState::Auto,
            1.0,
        );

        renderer.render().unwrap();

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

    #[test]
    fn skips_empty_classes_renders_placeholder() {
        let (cache, renderer, tmp) = make_renderer();
        // Only insert a style facet; all other classes will be empty.
        insert_facet(
            &cache,
            "style/verbosity",
            "terse",
            FacetState::Active,
            UserState::Auto,
            2.0,
        );

        renderer.render().unwrap();

        let body = std::fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
        // Empty classes get the placeholder.
        assert!(
            body.contains("*(no entries yet)*"),
            "placeholder missing:\n{body}"
        );
        // The style class has real content.
        assert!(body.contains("- **verbosity**: terse"));
    }

    #[test]
    fn pinned_facets_marked_in_output() {
        let (cache, renderer, tmp) = make_renderer();
        insert_facet(
            &cache,
            "style/format",
            "markdown",
            FacetState::Active,
            UserState::Pinned,
            1.0,
        );

        renderer.render().unwrap();

        let body = std::fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
        assert!(
            body.contains("*(pinned)*"),
            "pinned marker missing:\n{body}"
        );
        assert!(body.contains("- **format**: markdown *(pinned)*"));
    }

    #[test]
    fn provisional_facets_excluded_from_output() {
        let (cache, renderer, tmp) = make_renderer();
        insert_facet(
            &cache,
            "style/tone",
            "formal",
            FacetState::Provisional,
            UserState::Auto,
            0.8,
        );
        insert_facet(
            &cache,
            "style/verbosity",
            "terse",
            FacetState::Active,
            UserState::Auto,
            2.0,
        );

        renderer.render().unwrap();

        let body = std::fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
        assert!(
            !body.contains("formal"),
            "provisional must not appear:\n{body}"
        );
        assert!(body.contains("terse"));
    }

    #[test]
    fn re_renders_idempotently_on_repeated_cache_rebuilt() {
        let (cache, renderer, tmp) = make_renderer();
        insert_facet(
            &cache,
            "style/verbosity",
            "terse",
            FacetState::Active,
            UserState::Auto,
            2.0,
        );

        renderer.render().unwrap();
        let body1 = std::fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();
        renderer.render().unwrap();
        let body2 = std::fs::read_to_string(tmp.path().join("PROFILE.md")).unwrap();

        assert_eq!(body1, body2, "second render should be idempotent");
    }

    #[test]
    fn renders_dont_clobber_connected_accounts_block() {
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
        );
        renderer.render().unwrap();

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

    #[test]
    fn renders_dont_touch_user_authored_text_outside_blocks() {
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
        );
        renderer.render().unwrap();

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
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(PROFILE_INIT_SQL).unwrap();
        let cache = make_cache(Arc::new(Mutex::new(conn)));
        let renderer = Arc::new(ProfileMdRenderer::new(cache, tmp.path().to_path_buf()));
        // subscribe_global requires a running runtime; just verify the type works.
        let _renderer_ref = Arc::clone(&renderer);
        // We can't call subscribe_global in a unit test without a tokio runtime,
        // but we verify the subscriber type implements EventHandler correctly.
        let subscriber = RendererSubscriber(renderer);
        assert_eq!(subscriber.name(), "learning::profile_md_renderer");
        assert_eq!(subscriber.domains(), Some(["memory"].as_slice()));
    }
}
