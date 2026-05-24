//! Source-tree registry — thin wrapper around the generic
//! [`crate::openhuman::memory_tree::tree::registry::get_or_create_tree`]
//! that adds the source-specific `_source.md` on-disk mirror write after
//! every get-or-create call.

use anyhow::Result;

use crate::openhuman::config::Config;
use crate::openhuman::memory::util::redact::redact;
use crate::openhuman::memory_store::trees::types::{Tree, TreeKind};
use crate::openhuman::memory_tree::sources::file;
use crate::openhuman::memory_tree::tree::registry::get_or_create_tree;

/// Look up the source tree for `scope`, or create a new one.
///
/// Scope format convention (Phase 3a): use the ingested chunk's
/// `metadata.source_id` verbatim, so re-ingesting the same Slack channel
/// or Gmail account keeps appending to the same tree.
///
/// After every successful get-or-create the `_source.md` on-disk mirror
/// for this source is (re)written. The write is best-effort — a failure
/// is logged but does not abort the call.
pub fn get_or_create_source_tree(config: &Config, scope: &str) -> Result<Tree> {
    let scope_redacted = redact(scope);
    log::debug!("[sources::registry] get_or_create_source_tree scope={scope_redacted}");
    let tree = get_or_create_tree(config, TreeKind::Source, scope)?;
    if let Err(e) = file::write_source_file(config, &tree) {
        log::warn!("[sources::registry] write_source_file failed scope={scope_redacted} err={e:#}");
    }
    Ok(tree)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn test_config() -> (TempDir, Config) {
        let tmp = TempDir::new().unwrap();
        let mut cfg = Config::default();
        cfg.workspace_dir = tmp.path().to_path_buf();
        (tmp, cfg)
    }

    #[test]
    fn get_or_create_is_idempotent_on_scope() {
        let (_tmp, cfg) = test_config();
        let first = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        let second = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        assert_eq!(first.id, second.id);
        assert_eq!(first.kind, TreeKind::Source);
    }

    #[test]
    fn different_scopes_yield_different_trees() {
        let (_tmp, cfg) = test_config();
        let a = get_or_create_source_tree(&cfg, "slack:#eng").unwrap();
        let b = get_or_create_source_tree(&cfg, "gmail:user@example.com").unwrap();
        assert_ne!(a.id, b.id);
    }

    #[test]
    fn writes_source_file_on_create() {
        let (_tmp, cfg) = test_config();
        let tree = get_or_create_source_tree(&cfg, "gmail:user@example.com").unwrap();
        let path = file::source_file_path(&cfg, &tree.scope);
        assert!(path.exists(), "expected _source.md at {}", path.display());
    }
}
