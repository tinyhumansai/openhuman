//! Data migration helpers for Alphahuman.

use crate::alphahuman::config::Config;
use crate::alphahuman::memory::{self, Memory, MemoryCategory};
use anyhow::{bail, Context, Result};
use directories::UserDirs;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
struct SourceEntry {
    key: String,
    content: String,
    category: MemoryCategory,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct MigrationStats {
    pub from_sqlite: usize,
    pub from_markdown: usize,
    pub imported: usize,
    pub skipped_unchanged: usize,
    pub renamed_conflicts: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MigrationReport {
    pub source_workspace: PathBuf,
    pub target_workspace: PathBuf,
    pub dry_run: bool,
    pub stats: MigrationStats,
    pub warnings: Vec<String>,
}

pub async fn migrate_openclaw_memory(
    config: &Config,
    source_workspace: Option<PathBuf>,
    dry_run: bool,
) -> Result<MigrationReport> {
    let source_workspace = resolve_openclaw_workspace(source_workspace)?;
    if !source_workspace.exists() {
        bail!(
            "OpenClaw workspace not found at {}. Provide a valid source workspace.",
            source_workspace.display()
        );
    }

    if paths_equal(&source_workspace, &config.workspace_dir) {
        bail!("Source workspace matches current Alphahuman workspace; refusing self-migration");
    }

    let mut stats = MigrationStats::default();
    let entries = collect_source_entries(&source_workspace, &mut stats)?;
    let mut warnings = Vec::new();

    if entries.is_empty() {
        warnings.push(format!(
            "No importable memory found in {}",
            source_workspace.display()
        ));
        warnings.push("Checked for: memory/brain.db, MEMORY.md, memory/*.md".to_string());
        return Ok(MigrationReport {
            source_workspace,
            target_workspace: config.workspace_dir.clone(),
            dry_run,
            stats,
            warnings,
        });
    }

    if dry_run {
        return Ok(MigrationReport {
            source_workspace,
            target_workspace: config.workspace_dir.clone(),
            dry_run,
            stats,
            warnings,
        });
    }

    if let Some(backup_dir) = backup_target_memory(&config.workspace_dir)? {
        warnings.push(format!("Backup created: {}", backup_dir.display()));
    }

    let memory = target_memory_backend(config)?;

    for (idx, entry) in entries.into_iter().enumerate() {
        let mut key = entry.key.trim().to_string();
        if key.is_empty() {
            key = format!("openclaw_{idx}");
        }

        if let Some(existing) = memory.get(&key).await? {
            if existing.content.trim() == entry.content.trim() {
                stats.skipped_unchanged += 1;
                continue;
            }

            let renamed = next_available_key(memory.as_ref(), &key).await?;
            key = renamed;
            stats.renamed_conflicts += 1;
        }

        memory
            .store(&key, &entry.content, entry.category, None)
            .await?;
        stats.imported += 1;
    }

    Ok(MigrationReport {
        source_workspace,
        target_workspace: config.workspace_dir.clone(),
        dry_run,
        stats,
        warnings,
    })
}

fn target_memory_backend(config: &Config) -> Result<Box<dyn Memory>> {
    memory::create_memory_for_migration(&config.memory.backend, &config.workspace_dir)
}

fn collect_source_entries(
    source_workspace: &Path,
    stats: &mut MigrationStats,
) -> Result<Vec<SourceEntry>> {
    let mut entries = Vec::new();

    let sqlite_path = source_workspace.join("memory").join("brain.db");
    let sqlite_entries = read_openclaw_sqlite_entries(&sqlite_path)?;
    stats.from_sqlite = sqlite_entries.len();
    entries.extend(sqlite_entries);

    let markdown_entries = read_openclaw_markdown_entries(source_workspace)?;
    stats.from_markdown = markdown_entries.len();
    entries.extend(markdown_entries);

    // De-dup exact duplicates to make re-runs deterministic.
    let mut seen = HashSet::new();
    entries.retain(|entry| {
        let sig = format!("{}\u{0}{}\u{0}{}", entry.key, entry.content, entry.category);
        seen.insert(sig)
    });

    Ok(entries)
}

fn read_openclaw_sqlite_entries(db_path: &Path) -> Result<Vec<SourceEntry>> {
    if !db_path.exists() {
        return Ok(Vec::new());
    }

    let conn = Connection::open_with_flags(db_path, OpenFlags::SQLITE_OPEN_READ_ONLY)
        .with_context(|| format!("Failed to open source db {}", db_path.display()))?;

    let table_exists: Option<String> = conn
        .query_row(
            "SELECT name FROM sqlite_master WHERE type='table' AND name='memories' LIMIT 1",
            [],
            |row| row.get(0),
        )
        .optional()?;

    if table_exists.is_none() {
        return Ok(Vec::new());
    }

    let columns = table_columns(&conn, "memories")?;
    let key_expr = pick_column_expr(&columns, &["key", "id", "name"], "CAST(rowid AS TEXT)");
    let Some(content_expr) =
        pick_optional_column_expr(&columns, &["content", "value", "text", "memory"])
    else {
        bail!("OpenClaw memories table found but no content-like column was detected");
    };
    let category_expr = pick_column_expr(&columns, &["category", "kind", "type"], "'core'");

    let sql = format!(
        "SELECT {key_expr} AS key, {content_expr} AS content, {category_expr} AS category FROM memories"
    );

    let mut stmt = conn.prepare(&sql)?;
    let mut rows = stmt.query([])?;

    let mut entries = Vec::new();
    let mut idx = 0_usize;

    while let Some(row) = rows.next()? {
        let key: String = row
            .get(0)
            .unwrap_or_else(|_| format!("openclaw_sqlite_{idx}"));
        let content: String = row.get(1).unwrap_or_default();
        let category_raw: String = row.get(2).unwrap_or_else(|_| "core".to_string());

        if content.trim().is_empty() {
            continue;
        }

        entries.push(SourceEntry {
            key: normalize_key(&key, idx),
            content: content.trim().to_string(),
            category: parse_category(&category_raw),
        });

        idx += 1;
    }

    Ok(entries)
}

fn read_openclaw_markdown_entries(workspace: &Path) -> Result<Vec<SourceEntry>> {
    let mut entries = Vec::new();

    let top_level = workspace.join("MEMORY.md");
    if top_level.exists() {
        let content = fs::read_to_string(&top_level)
            .with_context(|| format!("Failed to read {}", top_level.display()))?;
        if !content.trim().is_empty() {
            entries.push(SourceEntry {
                key: "openclaw_memory_md".to_string(),
                content: content.trim().to_string(),
                category: MemoryCategory::Core,
            });
        }
    }

    let memory_dir = workspace.join("memory");
    if !memory_dir.exists() {
        return Ok(entries);
    }

    let mut idx = 0_usize;
    for entry in fs::read_dir(&memory_dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.extension().and_then(|s| s.to_str()) != Some("md") {
            continue;
        }

        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        if content.trim().is_empty() {
            continue;
        }

        let file_stem = path
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("openclaw");

        entries.push(SourceEntry {
            key: normalize_key(file_stem, idx),
            content: content.trim().to_string(),
            category: MemoryCategory::Core,
        });

        idx += 1;
    }

    Ok(entries)
}

fn resolve_openclaw_workspace(source: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = source {
        return Ok(path);
    }

    let Some(user_dirs) = UserDirs::new() else {
        bail!("Failed to determine user home directory");
    };

    Ok(user_dirs.home_dir().join(".openclaw").join("workspace"))
}

fn paths_equal(left: &Path, right: &Path) -> bool {
    if let (Ok(left), Ok(right)) = (left.canonicalize(), right.canonicalize()) {
        left == right
    } else {
        left == right
    }
}

fn normalize_key(raw: &str, idx: usize) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return format!("openclaw_{idx}");
    }

    trimmed
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .collect::<String>()
        .trim_matches('_')
        .to_string()
}

fn parse_category(raw: &str) -> MemoryCategory {
    match raw.trim().to_lowercase().as_str() {
        "core" => MemoryCategory::Core,
        "daily" => MemoryCategory::Daily,
        "conversation" => MemoryCategory::Conversation,
        "personal" => MemoryCategory::Custom("personal".to_string()),
        "project" => MemoryCategory::Custom("project".to_string()),
        "episode" => MemoryCategory::Custom("episode".to_string()),
        other => MemoryCategory::Custom(other.to_string()),
    }
}

fn backup_target_memory(workspace_dir: &Path) -> Result<Option<PathBuf>> {
    let mem_dir = workspace_dir.join("memory");
    let markdown = workspace_dir.join("MEMORY.md");
    let sqlite = mem_dir.join("brain.db");

    if !mem_dir.exists() && !markdown.exists() && !sqlite.exists() {
        return Ok(None);
    }

    let backup_dir = workspace_dir.join("memory_backup");
    fs::create_dir_all(&backup_dir)?;

    if markdown.exists() {
        let dest = backup_dir.join("MEMORY.md");
        fs::copy(&markdown, &dest).ok();
    }

    if sqlite.exists() {
        let dest = backup_dir.join("brain.db");
        fs::copy(&sqlite, &dest).ok();
    }

    if mem_dir.exists() {
        let dest_dir = backup_dir.join("memory");
        if !dest_dir.exists() {
            fs::create_dir_all(&dest_dir).ok();
        }
        for entry in fs::read_dir(&mem_dir)? {
            let entry = entry?;
            let path = entry.path();
            if path.extension().and_then(|s| s.to_str()) != Some("md") {
                continue;
            }
            let dest = dest_dir.join(
                path.file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("memory.md"),
            );
            fs::copy(&path, &dest).ok();
        }
    }

    Ok(Some(backup_dir))
}

fn table_columns(conn: &Connection, table: &str) -> Result<Vec<String>> {
    let mut stmt = conn.prepare(&format!("PRAGMA table_info({table})"))?;
    let mut rows = stmt.query([])?;

    let mut columns = Vec::new();
    while let Some(row) = rows.next()? {
        let name: String = row.get(1)?;
        columns.push(name);
    }

    Ok(columns)
}

fn pick_column_expr<'a>(
    columns: &'a [String],
    candidates: &[&'a str],
    fallback: &'a str,
) -> &'a str {
    for candidate in candidates {
        if columns.iter().any(|c| c.eq_ignore_ascii_case(candidate)) {
            return candidate;
        }
    }
    fallback
}

fn pick_optional_column_expr<'a>(
    columns: &'a [String],
    candidates: &[&'a str],
) -> Option<&'a str> {
    for candidate in candidates {
        if columns.iter().any(|c| c.eq_ignore_ascii_case(candidate)) {
            return Some(candidate);
        }
    }
    None
}

async fn next_available_key(memory: &dyn Memory, key: &str) -> Result<String> {
    let mut idx = 1u32;
    loop {
        let candidate = format!("{key}_{idx}");
        if memory.get(&candidate).await?.is_none() {
            return Ok(candidate);
        }
        idx += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_key_replaces_non_alnum() {
        let key = normalize_key("hello/world", 0);
        assert_eq!(key, "hello_world");
    }

    #[test]
    fn parse_category_defaults_to_core() {
        assert_eq!(
            parse_category("unknown"),
            MemoryCategory::Custom("unknown".to_string())
        );
    }
}
