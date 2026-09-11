//! Scope derivation for a configured source: which tree scope, and which
//! raw-archive id, a registry entry maps onto.
//!
//! # What came home and what did not (#5560)
//!
//! `tinymemory_core::sources::sync` was the whole ingest pipeline — the source
//! run, the tree rebuild, the re-embed queue and the Composio sync half — and
//! exactly one item of it was reached from production here:
//! [`derive_scopes`], called by `rpc::reconcile_rpc`. That one is not pipeline
//! at all. It reads the registry entry's `kind` and `url`, formats them through
//! `tinymemory_sources::readers::github` (an engine-neutral crate this host
//! already depends on), and for a Gmail connector scans
//! `<content root>/raw/gmail-*/_source.md` for the scope the archiver wrote.
//! Registry fields, a filesystem scan under a path the host owns, and no store
//! access — so it is host work that happened to live upstream, the same line
//! [`super::reconcile`] came home along.
//!
//! Everything else in that module stayed where it is. `sync_source` had no
//! caller left in `src/` — the sync the product runs goes over the bus through
//! `MemorySourceSync` — and porting the pipeline would move it *into* the host
//! rather than behind the module, which is the opposite of the point.

use crate::openhuman::config::Config;
use crate::openhuman::memory::sources::types::{MemorySourceEntry, SourceKind};

/// A source's tree scope paired with its raw-archive source id.
///
/// The two slugify to **different** directories for GitHub
/// (`github:owner/repo` vs `github.com/owner/repo`); conflating them makes
/// reconcile scan an empty directory while the real archive sits uncovered.
///
/// Not to be confused with `tinymemory_api::provider::types::SourceScope`,
/// which is the recall allowlist that crosses the bus. Same name, unrelated
/// shapes — check which one a call site means before moving it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SourceScope {
    /// Tree registry key, e.g. `"github:owner/repo"`.
    pub tree_scope: String,
    /// Raw-archive id whose slug names `raw/<slug>/`, e.g.
    /// `"github.com/owner/repo"`. Equal to `tree_scope` for sources that
    /// archive under their scope (gmail).
    pub archive_source_id: String,
}

/// Derive the tree scope(s) + raw-archive id(s) that a source maps to.
///
/// A verbatim port of the engine's function: same match arms, same
/// `gmail-` directory prefix, same `scope:` line parse, same empty answer for
/// every kind that has no raw archive to reconcile yet.
pub fn derive_scopes(source: &MemorySourceEntry, config: &Config) -> Vec<SourceScope> {
    use crate::openhuman::memory::sources::readers::github;

    match source.kind {
        SourceKind::GithubRepo => {
            let Some(url) = source.url.as_deref() else {
                return Vec::new();
            };
            match (
                github::repo_chunk_scope(url),
                github::repo_archive_source_id(url),
            ) {
                (Some(tree_scope), Some(archive_source_id)) => vec![SourceScope {
                    tree_scope,
                    archive_source_id,
                }],
                _ => Vec::new(),
            }
        }
        SourceKind::Composio => {
            // Composio sources scope by toolkit + connection email.
            // Gmail: "gmail:<slug_account_email>" — archive dir shares
            // the scope. Others: no raw archive to reconcile yet.
            let toolkit = source.toolkit.as_deref().unwrap_or("unknown");
            match toolkit {
                "gmail" | "GMAIL" => {
                    // The scope for gmail is "gmail:<slugified_email>".
                    // We scan the raw directory to find it.
                    let content_root = config.memory_tree_content_root();
                    let raw_dir = content_root.join("raw");
                    if let Ok(entries) = std::fs::read_dir(&raw_dir) {
                        entries
                            .filter_map(|e| e.ok())
                            .filter(|e| {
                                e.file_name()
                                    .to_str()
                                    .map(|n| n.starts_with("gmail-"))
                                    .unwrap_or(false)
                            })
                            .filter_map(|e| {
                                // Read _source.md to get the scope.
                                let source_md = e.path().join("_source.md");
                                let content = std::fs::read_to_string(&source_md).ok()?;
                                content.lines().find(|l| l.starts_with("scope:")).map(|l| {
                                    let scope = l
                                        .trim_start_matches("scope:")
                                        .trim()
                                        .trim_matches('"')
                                        .to_string();
                                    SourceScope {
                                        tree_scope: scope.clone(),
                                        archive_source_id: scope,
                                    }
                                })
                            })
                            .collect()
                    } else {
                        Vec::new()
                    }
                }
                _ => Vec::new(),
            }
        }
        _ => Vec::new(),
    }
}

#[cfg(test)]
#[path = "sync_tests.rs"]
mod tests;
