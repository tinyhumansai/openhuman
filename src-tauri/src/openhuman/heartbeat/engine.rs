use crate::openhuman::config::HeartbeatConfig;
use crate::openhuman::observability::{Observer, ObserverEvent};
use anyhow::Result;
use std::path::Path;
use std::sync::Arc;
use tokio::time::{self, Duration};
use tracing::{info, warn};

/// Heartbeat engine — reads HEARTBEAT.md and executes tasks periodically
pub struct HeartbeatEngine {
    config: HeartbeatConfig,
    workspace_dir: std::path::PathBuf,
    observer: Arc<dyn Observer>,
}

impl HeartbeatEngine {
    pub fn new(
        config: HeartbeatConfig,
        workspace_dir: std::path::PathBuf,
        observer: Arc<dyn Observer>,
    ) -> Self {
        Self {
            config,
            workspace_dir,
            observer,
        }
    }

    /// Start the heartbeat loop (runs until cancelled)
    pub async fn run(&self) -> Result<()> {
        if !self.config.enabled {
            info!("Heartbeat disabled");
            return Ok(());
        }

        let interval_mins = self.config.interval_minutes.max(5);
        info!("💓 Heartbeat started: every {} minutes", interval_mins);

        let mut interval = time::interval(Duration::from_secs(u64::from(interval_mins) * 60));

        loop {
            interval.tick().await;
            self.observer.record_event(&ObserverEvent::HeartbeatTick);

            match self.tick().await {
                Ok(tasks) => {
                    if tasks > 0 {
                        info!("💓 Heartbeat: processed {} tasks", tasks);
                    }
                }
                Err(e) => {
                    warn!("💓 Heartbeat error: {}", e);
                    self.observer.record_event(&ObserverEvent::Error {
                        component: "heartbeat".into(),
                        message: e.to_string(),
                    });
                }
            }
        }
    }

    /// Single heartbeat tick — read HEARTBEAT.md and return task count
    async fn tick(&self) -> Result<usize> {
        Ok(self.collect_tasks().await?.len())
    }

    /// Read HEARTBEAT.md and return all parsed tasks.
    pub async fn collect_tasks(&self) -> Result<Vec<String>> {
        let heartbeat_path = self.workspace_dir.join("HEARTBEAT.md");
        if !heartbeat_path.exists() {
            return Ok(Vec::new());
        }
        let content = tokio::fs::read_to_string(&heartbeat_path).await?;
        Ok(Self::parse_tasks(&content))
    }

    /// Parse tasks from HEARTBEAT.md (lines starting with `- `)
    fn parse_tasks(content: &str) -> Vec<String> {
        content
            .lines()
            .filter_map(|line| {
                let trimmed = line.trim();
                trimmed.strip_prefix("- ").map(ToString::to_string)
            })
            .collect()
    }

    /// Create a default HEARTBEAT.md if it doesn't exist
    pub async fn ensure_heartbeat_file(workspace_dir: &Path) -> Result<()> {
        let path = workspace_dir.join("HEARTBEAT.md");
        if !path.exists() {
            let default = "# Periodic Tasks\n\n\
                           # Add tasks below (one per line, starting with `- `)\n\
                           # The agent will check this file on each heartbeat tick.\n\
                           #\n\
                           # Examples:\n\
                           # - Check my email for important messages\n\
                           # - Review my calendar for upcoming events\n\
                           # - Check the weather forecast\n";
            tokio::fs::write(&path, default).await?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_tasks_basic() {
        let content = "# Tasks\n\n- Check email\n- Review calendar\nNot a task\n- Third task";
        let tasks = HeartbeatEngine::parse_tasks(content);
        assert_eq!(tasks.len(), 3);
        assert_eq!(tasks[0], "Check email");
        assert_eq!(tasks[1], "Review calendar");
        assert_eq!(tasks[2], "Third task");
    }

    #[test]
    fn parse_tasks_empty_content() {
        assert!(HeartbeatEngine::parse_tasks("").is_empty());
    }

    #[test]
    fn parse_tasks_only_comments() {
        let tasks = HeartbeatEngine::parse_tasks("# No tasks here\n\nJust comments\n# Another");
        assert!(tasks.is_empty());
    }

    #[test]
    fn parse_tasks_with_leading_whitespace() {
        let content = "  - Indented task\n\t- Tab indented";
        let tasks = HeartbeatEngine::parse_tasks(content);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0], "Indented task");
        assert_eq!(tasks[1], "Tab indented");
    }

    #[test]
    fn parse_tasks_dash_without_space_ignored() {
        let content = "- Real task\n-\n- Another";
        let tasks = HeartbeatEngine::parse_tasks(content);
        // "-" trimmed = "-", does NOT start with "- " => skipped
        // "- Real task" => "Real task"
        // "- Another" => "Another"
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0], "Real task");
        assert_eq!(tasks[1], "Another");
    }

    #[test]
    fn parse_tasks_trailing_space_bullet_trimmed_to_dash() {
        // "- " trimmed becomes "-" (trim removes trailing space)
        // "-" does NOT start with "- " => skipped
        let content = "- ";
        let tasks = HeartbeatEngine::parse_tasks(content);
        assert_eq!(tasks.len(), 0);
    }

    #[test]
    fn parse_tasks_bullet_with_content_after_spaces() {
        // "- hello  " trimmed becomes "- hello" => starts_with "- " => "hello"
        let content = "- hello  ";
        let tasks = HeartbeatEngine::parse_tasks(content);
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0], "hello");
    }

    #[test]
    fn parse_tasks_unicode() {
        let content = "- Check email 📧\n- Review calendar 📅\n- 日本語タスク";
        let tasks = HeartbeatEngine::parse_tasks(content);
        assert_eq!(tasks.len(), 3);
        assert!(tasks[0].contains("📧"));
        assert!(tasks[2].contains("日本語"));
    }

    #[test]
    fn parse_tasks_mixed_markdown() {
        let content = "# Periodic Tasks\n\n## Quick\n- Task A\n\n## Long\n- Task B\n\n* Not a dash bullet\n1. Not numbered";
        let tasks = HeartbeatEngine::parse_tasks(content);
        assert_eq!(tasks.len(), 2);
        assert_eq!(tasks[0], "Task A");
        assert_eq!(tasks[1], "Task B");
    }

    #[test]
    fn parse_tasks_single_task() {
        let tasks = HeartbeatEngine::parse_tasks("- Only one");
        assert_eq!(tasks.len(), 1);
        assert_eq!(tasks[0], "Only one");
    }

    #[test]
    fn parse_tasks_many_tasks() {
        let content: String = (0..100).fold(String::new(), |mut s, i| {
            use std::fmt::Write;
            let _ = writeln!(s, "- Task {i}");
            s
        });
        let tasks = HeartbeatEngine::parse_tasks(&content);
        assert_eq!(tasks.len(), 100);
        assert_eq!(tasks[99], "Task 99");
    }

    #[tokio::test]
    async fn ensure_heartbeat_file_creates_file() {
        let dir = std::env::temp_dir().join("openhuman_test_heartbeat");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        HeartbeatEngine::ensure_heartbeat_file(&dir).await.unwrap();

        let path = dir.join("HEARTBEAT.md");
        assert!(path.exists());
        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert!(content.contains("Periodic Tasks"));

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn ensure_heartbeat_file_does_not_overwrite() {
        let dir = std::env::temp_dir().join("openhuman_test_heartbeat_no_overwrite");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let path = dir.join("HEARTBEAT.md");
        tokio::fs::write(&path, "- My custom task").await.unwrap();

        HeartbeatEngine::ensure_heartbeat_file(&dir).await.unwrap();

        let content = tokio::fs::read_to_string(&path).await.unwrap();
        assert_eq!(content, "- My custom task");

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn tick_returns_zero_when_no_file() {
        let dir = std::env::temp_dir().join("openhuman_test_tick_no_file");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        let observer: Arc<dyn Observer> = Arc::new(crate::openhuman::observability::NoopObserver);
        let engine = HeartbeatEngine::new(
            HeartbeatConfig {
                enabled: true,
                interval_minutes: 30,
            },
            dir.clone(),
            observer,
        );
        let count = engine.tick().await.unwrap();
        assert_eq!(count, 0);

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn tick_counts_tasks_from_file() {
        let dir = std::env::temp_dir().join("openhuman_test_tick_count");
        let _ = tokio::fs::remove_dir_all(&dir).await;
        tokio::fs::create_dir_all(&dir).await.unwrap();

        tokio::fs::write(dir.join("HEARTBEAT.md"), "- A\n- B\n- C")
            .await
            .unwrap();

        let observer: Arc<dyn Observer> = Arc::new(crate::openhuman::observability::NoopObserver);
        let engine = HeartbeatEngine::new(
            HeartbeatConfig {
                enabled: true,
                interval_minutes: 30,
            },
            dir.clone(),
            observer,
        );
        let count = engine.tick().await.unwrap();
        assert_eq!(count, 3);

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn run_returns_immediately_when_disabled() {
        let observer: Arc<dyn Observer> = Arc::new(crate::openhuman::observability::NoopObserver);
        let engine = HeartbeatEngine::new(
            HeartbeatConfig {
                enabled: false,
                interval_minutes: 30,
            },
            std::env::temp_dir(),
            observer,
        );
        // Should return Ok immediately, not loop forever
        let result = engine.run().await;
        assert!(result.is_ok());
    }
}
