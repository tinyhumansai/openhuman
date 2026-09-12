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

use crate::core::bus::BUS;
use crate::core::events::DomainEvent;
use crate::openhuman::agent::learning::cache::FacetCache;
use crate::openhuman::integrations::composio::profile_md::replace_managed_block;
use tinybus::EventHandler;
use tinybus::SubscriptionHandle;
use tinymemory_api::provider::UserState;

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
    /// Async since the facet read became a driver call. The
    /// `spawn_blocking` the subscriber used to wrap this in is gone with it —
    /// there is no in-process SQLite left to keep off the executor.
    pub async fn render(&self) -> anyhow::Result<()> {
        tracing::debug!("[learning::profile_md_renderer] render triggered — reading active facets");

        let active_facets = self.cache.list_active().await?;

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
        BUS.subscribe(Arc::new(RendererSubscriber(renderer)))
    }
}

// ── Event subscriber ─────────────────────────────────────────────────────────

struct RendererSubscriber(Arc<ProfileMdRenderer>);

#[async_trait]
impl EventHandler<DomainEvent> for RendererSubscriber {
    fn name(&self) -> &str {
        "learning::profile_md_renderer"
    }

    fn domains(&self) -> Option<&[&str]> {
        Some(&["memory"])
    }

    async fn handle(&self, event: &DomainEvent) {
        if let DomainEvent::CacheRebuilt { .. } = event {
            // Awaited directly. This used to be `spawn_blocking`, because the
            // facet read was in-process SQLite; it is a driver call now, so
            // there is nothing blocking to move off the executor. The file
            // write that remains is small and bounded.
            if let Err(e) = self.0.render().await {
                tracing::warn!(
                    "[learning::profile_md_renderer] render on CacheRebuilt failed: {e:#}"
                );
            }
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
#[path = "profile_md_renderer_tests.rs"]
mod tests;
