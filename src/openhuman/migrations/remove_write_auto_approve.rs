//! Migration 4 -> 5: remove write tools from `autonomy.auto_approve`.
//!
//! A short-lived v4 default/migration added `file_write` and `edit_file` to
//! `auto_approve`, which made Supervised mode skip its ask-before-edit prompt.
//! The v3 -> v4 migration no longer adds them, but workspaces that already
//! persisted schema_version 4 still need their config scrubbed.

use crate::openhuman::config::Config;

const WRITE_AUTO_APPROVE_TOOLS: &[&str] = &["file_write", "edit_file"];

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct MigrationStats {
    pub auto_approve_removed: usize,
}

pub fn run(config: &mut Config) -> anyhow::Result<MigrationStats> {
    let before_len = config.autonomy.auto_approve.len();
    config.autonomy.auto_approve.retain(|tool| {
        !WRITE_AUTO_APPROVE_TOOLS
            .iter()
            .any(|blocked| tool == blocked)
    });

    let stats = MigrationStats {
        auto_approve_removed: before_len.saturating_sub(config.autonomy.auto_approve.len()),
    };

    log::info!(
        "[migrations][remove-write-auto-approve] done auto_approve_removed={}",
        stats.auto_approve_removed
    );

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::openhuman::config::Config;

    #[test]
    fn removes_write_tools_from_auto_approve() {
        let mut config = Config::default();
        config.autonomy.auto_approve = vec![
            "file_read".into(),
            "file_write".into(),
            "edit_file".into(),
            "glob".into(),
        ];

        let stats = run(&mut config).expect("migration should succeed");

        assert_eq!(stats.auto_approve_removed, 2);
        assert_eq!(
            config.autonomy.auto_approve,
            vec!["file_read".to_string(), "glob".to_string()]
        );
    }

    #[test]
    fn removes_write_tools_even_when_mixed() {
        let mut config = Config::default();
        config.autonomy.auto_approve = vec!["file_write".into()];

        run(&mut config).expect("migration should succeed");

        assert!(config.autonomy.auto_approve.is_empty());
    }
}
