use super::sqlite::SqliteMemory;
use super::traits::{Memory, MemoryCategory, MemoryEntry};
use async_trait::async_trait;
use chrono::Local;
use parking_lot::Mutex;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use tokio::process::Command;
use tokio::time::timeout;

pub struct LucidMemory {
    local: SqliteMemory,
    lucid_cmd: String,
    token_budget: usize,
    workspace_dir: PathBuf,
    recall_timeout: Duration,
    store_timeout: Duration,
    local_hit_threshold: usize,
    failure_cooldown: Duration,
    last_failure_at: Mutex<Option<Instant>>,
}

impl LucidMemory {
    const DEFAULT_LUCID_CMD: &'static str = "lucid";
    const DEFAULT_TOKEN_BUDGET: usize = 200;
    // Lucid UI cold start can exceed 120ms on slower machines, which causes
    // avoidable fallback to local-only memory and premature cooldown.
    const DEFAULT_RECALL_TIMEOUT_MS: u64 = 500;
    const DEFAULT_STORE_TIMEOUT_MS: u64 = 800;
    const DEFAULT_LOCAL_HIT_THRESHOLD: usize = 3;
    const DEFAULT_FAILURE_COOLDOWN_MS: u64 = 15_000;

    pub fn new(workspace_dir: &Path, local: SqliteMemory) -> Self {
        let lucid_cmd = std::env::var("ALPHAHUMAN_LUCID_CMD")
            .unwrap_or_else(|_| Self::DEFAULT_LUCID_CMD.to_string());

        let token_budget = std::env::var("ALPHAHUMAN_LUCID_BUDGET")
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .filter(|v| *v > 0)
            .unwrap_or(Self::DEFAULT_TOKEN_BUDGET);

        let recall_timeout = Self::read_env_duration_ms(
            "ALPHAHUMAN_LUCID_RECALL_TIMEOUT_MS",
            Self::DEFAULT_RECALL_TIMEOUT_MS,
            20,
        );
        let store_timeout = Self::read_env_duration_ms(
            "ALPHAHUMAN_LUCID_STORE_TIMEOUT_MS",
            Self::DEFAULT_STORE_TIMEOUT_MS,
            50,
        );
        let local_hit_threshold = Self::read_env_usize(
            "ALPHAHUMAN_LUCID_LOCAL_HIT_THRESHOLD",
            Self::DEFAULT_LOCAL_HIT_THRESHOLD,
            1,
        );
        let failure_cooldown = Self::read_env_duration_ms(
            "ALPHAHUMAN_LUCID_FAILURE_COOLDOWN_MS",
            Self::DEFAULT_FAILURE_COOLDOWN_MS,
            100,
        );

        Self {
            local,
            lucid_cmd,
            token_budget,
            workspace_dir: workspace_dir.to_path_buf(),
            recall_timeout,
            store_timeout,
            local_hit_threshold,
            failure_cooldown,
            last_failure_at: Mutex::new(None),
        }
    }

    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    fn with_options(
        workspace_dir: &Path,
        local: SqliteMemory,
        lucid_cmd: String,
        token_budget: usize,
        local_hit_threshold: usize,
        recall_timeout: Duration,
        store_timeout: Duration,
        failure_cooldown: Duration,
    ) -> Self {
        Self {
            local,
            lucid_cmd,
            token_budget,
            workspace_dir: workspace_dir.to_path_buf(),
            recall_timeout,
            store_timeout,
            local_hit_threshold: local_hit_threshold.max(1),
            failure_cooldown,
            last_failure_at: Mutex::new(None),
        }
    }

    fn read_env_usize(name: &str, default: usize, min: usize) -> usize {
        std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<usize>().ok())
            .map_or(default, |v| v.max(min))
    }

    fn read_env_duration_ms(name: &str, default_ms: u64, min_ms: u64) -> Duration {
        let millis = std::env::var(name)
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .map_or(default_ms, |v| v.max(min_ms));
        Duration::from_millis(millis)
    }

    fn in_failure_cooldown(&self) -> bool {
        let guard = self.last_failure_at.lock();
        guard
            .as_ref()
            .is_some_and(|last| last.elapsed() < self.failure_cooldown)
    }

    fn mark_failure_now(&self) {
        let mut guard = self.last_failure_at.lock();
        *guard = Some(Instant::now());
    }

    fn clear_failure(&self) {
        let mut guard = self.last_failure_at.lock();
        *guard = None;
    }

    fn to_lucid_type(category: &MemoryCategory) -> &'static str {
        match category {
            MemoryCategory::Core => "decision",
            MemoryCategory::Daily => "context",
            MemoryCategory::Conversation => "conversation",
            MemoryCategory::Custom(_) => "learning",
        }
    }

    fn to_memory_category(label: &str) -> MemoryCategory {
        let normalized = label.to_lowercase();
        if normalized.contains("visual") {
            return MemoryCategory::Custom("visual".to_string());
        }

        match normalized.as_str() {
            "decision" | "learning" | "solution" => MemoryCategory::Core,
            "context" | "conversation" => MemoryCategory::Conversation,
            "bug" => MemoryCategory::Daily,
            other => MemoryCategory::Custom(other.to_string()),
        }
    }

    fn merge_results(
        primary_results: Vec<MemoryEntry>,
        secondary_results: Vec<MemoryEntry>,
        limit: usize,
    ) -> Vec<MemoryEntry> {
        if limit == 0 {
            return Vec::new();
        }

        let mut merged = Vec::new();
        let mut seen = HashSet::new();

        for entry in primary_results.into_iter().chain(secondary_results) {
            let signature = format!(
                "{}\u{0}{}",
                entry.key.to_lowercase(),
                entry.content.to_lowercase()
            );

            if seen.insert(signature) {
                merged.push(entry);
                if merged.len() >= limit {
                    break;
                }
            }
        }

        merged
    }

    fn parse_lucid_context(raw: &str) -> Vec<MemoryEntry> {
        let mut in_context_block = false;
        let mut entries = Vec::new();
        let now = Local::now().to_rfc3339();

        for line in raw.lines().map(str::trim) {
            if line == "<lucid-context>" {
                in_context_block = true;
                continue;
            }

            if line == "</lucid-context>" {
                break;
            }

            if !in_context_block || line.is_empty() {
                continue;
            }

            let Some(rest) = line.strip_prefix("- [") else {
                continue;
            };

            let Some((label, content_part)) = rest.split_once(']') else {
                continue;
            };

            let content = content_part.trim();
            if content.is_empty() {
                continue;
            }

            let rank = entries.len();
            entries.push(MemoryEntry {
                id: format!("lucid:{rank}"),
                key: format!("lucid_{rank}"),
                content: content.to_string(),
                category: Self::to_memory_category(label.trim()),
                timestamp: now.clone(),
                session_id: None,
                score: Some((1.0 - rank as f64 * 0.05).max(0.1)),
            });
        }

        entries
    }

    async fn run_lucid_command_raw(
        lucid_cmd: &str,
        args: &[String],
        timeout_window: Duration,
    ) -> anyhow::Result<String> {
        let mut cmd = if Path::new(lucid_cmd)
            .extension()
            .is_some_and(|ext| ext == "sh")
        {
            let mut cmd = Command::new("bash");
            cmd.arg(lucid_cmd);
            cmd
        } else {
            Command::new(lucid_cmd)
        };

        cmd.args(args);

        let output = timeout(timeout_window, cmd.output()).await.map_err(|_| {
            anyhow::anyhow!(
                "lucid command timed out after {}ms",
                timeout_window.as_millis()
            )
        })??;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("lucid command failed: {stderr}");
        }

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    async fn run_lucid_command(
        &self,
        args: &[String],
        timeout_window: Duration,
    ) -> anyhow::Result<String> {
        Self::run_lucid_command_raw(&self.lucid_cmd, args, timeout_window).await
    }

    fn build_store_args(&self, key: &str, content: &str, category: &MemoryCategory) -> Vec<String> {
        let payload = format!("{key}: {content}");
        vec![
            "store".to_string(),
            payload,
            format!("--type={}", Self::to_lucid_type(category)),
            format!("--project={}", self.workspace_dir.display()),
        ]
    }

    fn build_recall_args(&self, query: &str) -> Vec<String> {
        vec![
            "context".to_string(),
            query.to_string(),
            format!("--budget={}", self.token_budget),
            format!("--project={}", self.workspace_dir.display()),
        ]
    }

    async fn sync_to_lucid_async(&self, key: &str, content: &str, category: &MemoryCategory) {
        let args = self.build_store_args(key, content, category);
        if let Err(error) = self.run_lucid_command(&args, self.store_timeout).await {
            tracing::debug!(
                command = %self.lucid_cmd,
                error = %error,
                "Lucid store sync failed; sqlite remains authoritative"
            );
        }
    }

    async fn recall_from_lucid(&self, query: &str) -> anyhow::Result<Vec<MemoryEntry>> {
        let args = self.build_recall_args(query);
        let output = self.run_lucid_command(&args, self.recall_timeout).await?;
        Ok(Self::parse_lucid_context(&output))
    }
}

#[async_trait]
impl Memory for LucidMemory {
    fn name(&self) -> &str {
        "lucid"
    }

    async fn store(
        &self,
        key: &str,
        content: &str,
        category: MemoryCategory,
        session_id: Option<&str>,
    ) -> anyhow::Result<()> {
        self.local
            .store(key, content, category.clone(), session_id)
            .await?;
        self.sync_to_lucid_async(key, content, &category).await;
        Ok(())
    }

    async fn recall(
        &self,
        query: &str,
        limit: usize,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        let local_results = self.local.recall(query, limit, session_id).await?;
        if limit == 0
            || local_results.len() >= limit
            || local_results.len() >= self.local_hit_threshold
        {
            return Ok(local_results);
        }

        if self.in_failure_cooldown() {
            return Ok(local_results);
        }

        match self.recall_from_lucid(query).await {
            Ok(lucid_results) if !lucid_results.is_empty() => {
                self.clear_failure();
                Ok(Self::merge_results(local_results, lucid_results, limit))
            }
            Ok(_) => {
                self.clear_failure();
                Ok(local_results)
            }
            Err(error) => {
                self.mark_failure_now();
                tracing::debug!(
                    command = %self.lucid_cmd,
                    error = %error,
                    "Lucid context unavailable; using local sqlite results"
                );
                Ok(local_results)
            }
        }
    }

    async fn get(&self, key: &str) -> anyhow::Result<Option<MemoryEntry>> {
        self.local.get(key).await
    }

    async fn list(
        &self,
        category: Option<&MemoryCategory>,
        session_id: Option<&str>,
    ) -> anyhow::Result<Vec<MemoryEntry>> {
        self.local.list(category, session_id).await
    }

    async fn forget(&self, key: &str) -> anyhow::Result<bool> {
        self.local.forget(key).await
    }

    async fn count(&self) -> anyhow::Result<usize> {
        self.local.count().await
    }

    async fn health_check(&self) -> bool {
        self.local.health_check().await
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::TempDir;

    fn write_fake_lucid_script(dir: &Path) -> String {
        let script_path = dir.join("fake-lucid.sh");
        let script = r#"#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "store" ]]; then
  echo '{"success":true,"id":"mem_1"}'
  exit 0
fi

if [[ "${1:-}" == "context" ]]; then
  cat <<'EOF'
<lucid-context>
Auth context snapshot
- [decision] Use token refresh middleware
- [context] Working in src/auth.rs
</lucid-context>
EOF
  exit 0
fi

echo "unsupported command" >&2
exit 1
"#;

        fs::write(&script_path, script).unwrap();
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
        script_path.display().to_string()
    }

    fn write_delayed_lucid_script(dir: &Path) -> String {
        let script_path = dir.join("delayed-lucid.sh");
        let script = r#"#!/usr/bin/env bash
set -euo pipefail

if [[ "${1:-}" == "store" ]]; then
  echo '{"success":true,"id":"mem_1"}'
  exit 0
fi

if [[ "${1:-}" == "context" ]]; then
  # Simulate a cold start delay that stays well below the recall timeout.
  sleep 0.1
  cat <<'EOF'
<lucid-context>
- [decision] Delayed token refresh guidance
</lucid-context>
EOF
  exit 0
fi

echo "unsupported command" >&2
exit 1
"#;

        fs::write(&script_path, script).unwrap();
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
        script_path.display().to_string()
    }

    fn write_probe_lucid_script(dir: &Path, marker_path: &Path) -> String {
        let script_path = dir.join("probe-lucid.sh");
        let marker = marker_path.display().to_string();
        let script = format!(
            r#"#!/usr/bin/env bash
set -euo pipefail

if [[ "${{1:-}}" == "store" ]]; then
  echo '{{"success":true,"id":"mem_store"}}'
  exit 0
fi

if [[ "${{1:-}}" == "context" ]]; then
  printf 'context\n' >> "{marker}"
  cat <<'EOF'
<lucid-context>
- [decision] should not be used when local hits are enough
</lucid-context>
EOF
  exit 0
fi

echo "unsupported command" >&2
exit 1
"#
        );

        fs::write(&script_path, script).unwrap();
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
        script_path.display().to_string()
    }

    fn test_memory(workspace: &Path, cmd: String) -> LucidMemory {
        let sqlite = SqliteMemory::new(workspace).unwrap();
        LucidMemory::with_options(
            workspace,
            sqlite,
            cmd,
            200,
            3,
            Duration::from_millis(500),
            Duration::from_secs(1),
            Duration::from_secs(2),
        )
    }

    #[tokio::test]
    async fn lucid_name() {
        let tmp = TempDir::new().unwrap();
        let memory = test_memory(tmp.path(), "nonexistent-lucid-binary".to_string());
        assert_eq!(memory.name(), "lucid");
    }

    #[tokio::test]
    async fn store_succeeds_when_lucid_missing() {
        let tmp = TempDir::new().unwrap();
        let memory = test_memory(tmp.path(), "nonexistent-lucid-binary".to_string());

        memory
            .store("lang", "User prefers Rust", MemoryCategory::Core, None)
            .await
            .unwrap();

        let entry = memory.get("lang").await.unwrap();
        assert!(entry.is_some());
        assert_eq!(entry.unwrap().content, "User prefers Rust");
    }

    #[tokio::test]
    async fn recall_merges_lucid_and_local_results() {
        let tmp = TempDir::new().unwrap();
        let fake_cmd = write_fake_lucid_script(tmp.path());
        let memory = test_memory(tmp.path(), fake_cmd);

        memory
            .store(
                "local_note",
                "Local sqlite auth fallback note",
                MemoryCategory::Core,
                None,
            )
            .await
            .unwrap();

        let entries = memory.recall("auth", 5, None).await.unwrap();

        assert!(entries
            .iter()
            .any(|e| e.content.contains("Local sqlite auth fallback note")));
        assert!(entries.iter().any(|e| e.content.contains("token refresh")));
    }

    #[tokio::test]
    async fn recall_handles_lucid_cold_start_delay_within_timeout() {
        let tmp = TempDir::new().unwrap();
        let delayed_cmd = write_delayed_lucid_script(tmp.path());
        let memory = test_memory(tmp.path(), delayed_cmd);

        memory
            .store(
                "local_note",
                "Local sqlite auth fallback note",
                MemoryCategory::Core,
                None,
            )
            .await
            .unwrap();

        let entries = memory.recall("auth", 5, None).await.unwrap();

        assert!(entries
            .iter()
            .any(|e| e.content.contains("Local sqlite auth fallback note")));
        assert!(entries
            .iter()
            .any(|e| e.content.contains("Delayed token refresh guidance")));
    }

    #[tokio::test]
    async fn recall_skips_lucid_when_local_hits_are_enough() {
        let tmp = TempDir::new().unwrap();
        let marker = tmp.path().join("context_calls.log");
        let probe_cmd = write_probe_lucid_script(tmp.path(), &marker);

        let sqlite = SqliteMemory::new(tmp.path()).unwrap();
        let memory = LucidMemory::with_options(
            tmp.path(),
            sqlite,
            probe_cmd,
            200,
            1,
            Duration::from_millis(500),
            Duration::from_millis(400),
            Duration::from_secs(2),
        );

        memory
            .store(
                "pref",
                "Rust should stay local-first",
                MemoryCategory::Core,
                None,
            )
            .await
            .unwrap();

        let entries = memory.recall("rust", 5, None).await.unwrap();
        assert!(entries
            .iter()
            .any(|e| e.content.contains("Rust should stay local-first")));

        let context_calls = tokio::fs::read_to_string(&marker).await.unwrap_or_default();
        assert!(
            context_calls.trim().is_empty(),
            "Expected local-hit short-circuit; got calls: {context_calls}"
        );
    }

    fn write_failing_lucid_script(dir: &Path, marker_path: &Path) -> String {
        let script_path = dir.join("failing-lucid.sh");
        let marker = marker_path.display().to_string();
        let script = format!(
            r#"#!/usr/bin/env bash
set -euo pipefail

if [[ "${{1:-}}" == "store" ]]; then
  echo '{{"success":true,"id":"mem_store"}}'
  exit 0
fi

if [[ "${{1:-}}" == "context" ]]; then
  printf 'context\n' >> "{marker}"
  echo "simulated lucid failure" >&2
  exit 1
fi

echo "unsupported command" >&2
exit 1
"#
        );

        fs::write(&script_path, script).unwrap();
        let mut perms = fs::metadata(&script_path).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&script_path, perms).unwrap();
        script_path.display().to_string()
    }

    #[tokio::test]
    async fn failure_cooldown_avoids_repeated_lucid_calls() {
        let tmp = TempDir::new().unwrap();
        let marker = tmp.path().join("failing_context_calls.log");
        let failing_cmd = write_failing_lucid_script(tmp.path(), &marker);

        let sqlite = SqliteMemory::new(tmp.path()).unwrap();
        let memory = LucidMemory::with_options(
            tmp.path(),
            sqlite,
            failing_cmd,
            200,
            99,
            Duration::from_millis(500),
            Duration::from_millis(400),
            Duration::from_secs(5),
        );

        let first = memory.recall("auth", 5, None).await.unwrap();
        let second = memory.recall("auth", 5, None).await.unwrap();

        assert!(first.is_empty());
        assert!(second.is_empty());

        let calls = tokio::fs::read_to_string(&marker).await.unwrap_or_default();
        assert_eq!(calls.lines().count(), 1);
    }
}
