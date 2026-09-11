use crate::openhuman::config::Config;
use crate::openhuman::memory::{Memory, MemoryCategory};
use anyhow::{bail, Context, Result};
use directories::UserDirs;
use rusqlite::{Connection, OpenFlags, OptionalExtension};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;

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
        bail!("Source workspace matches current OpenHuman workspace; refusing self-migration");
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

        if let Some(existing) = memory.get("", &key).await? {
            if existing.content.trim() == entry.content.trim() {
                stats.skipped_unchanged += 1;
                continue;
            }

            let renamed = next_available_key(memory.as_ref(), &key).await?;
            key = renamed;
            stats.renamed_conflicts += 1;
        }

        memory
            .store("", &key, &entry.content, entry.category, None)
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

/// The memory the import writes into.
///
/// The bound driver, through [`DriverMemory`] — the same store the engine
/// constructor this replaced opened. That equivalence is checked rather than
/// assumed: `create_memory_for_migration` bottoms out in
/// `create_memory_full(..., workspace_dir, "memory")`, and
/// `binding::for_workspace` resolves `for_subtree(workspace_dir, "memory")`.
/// Same workspace, same subtree.
///
/// # Why the capture budget does not bite here
///
/// This note used to say the contract had no door for an import, because
/// `MemoryCore::store` is reached through `MemoryGuard`, which applies
/// `MemoryHooksConfig::capture_max_chars` to every body — and an imported
/// `MEMORY.md` entry is routinely longer than that, so routing an import
/// through the guard would silently truncate the user's own memories.
///
/// The budget is real, and it is enforced host-side in
/// `memory::guard::policy`. What the note missed is that `DriverMemory` does
/// not go through the guard at all: it wraps `binding.provider()`, which is the
/// **unguarded** driver — the binding keeps the guarded one separately behind
/// `binding.guard()`. So an import writes full bodies here exactly as the
/// engine constructor did, and no policy has to be special-cased to allow it.
///
/// # A null driver is refused, not imported into
///
/// `binding` answers the null driver in three situations: `[subsystems.memory]
/// driver = "null"` is configured; a configured driver failed to bind and fell
/// back; and — the one this note used to miss — a driver was *admitted* as a
/// module but this build has no module to bind, so `binding::module_provider`
/// substituted the null provider under `#[cfg(not(feature = "modules"))]`.
/// In all three, every write below is discarded. An import that reports
/// "migrated N entries" having written none is silent data loss of the worst
/// kind — the source workspace may be deleted on the strength of that report,
/// and the user only discovers it later, with nothing left to re-run against.
/// So this refuses up front and names which of the three it was.
///
/// Telling the third case apart needs what was *admitted*, not what was bound.
/// `admit` is pure config and never saw the feature flag, so it answers
/// `Module` for the configured `tinycortex`; `module_provider` then binds the
/// null provider, and the binding reports `class = Null` with **no**
/// `fallback`, since nothing refused. So `admitted != bound` is the signal, and
/// it is the only one that works: `driver_id` alone does not separate this from
/// a driver deliberately given `class = "null"`, because `admit` accepts a
/// non-built-in id with that class (the `built_in_class` check is skipped for
/// ids that are not built in) and returns it verbatim. Keying on the id would
/// tell a user with a working modules build that their `modules` feature was
/// off — the same class of misdirection this whole note exists to prevent.
///
/// Keying on the class alone was the original bug: it put the modules-off case
/// in the configured-null arm and told a user whose config says `driver =
/// "tinycortex"` that they had set it to `"null"`, sending them to edit a line
/// that already said the opposite.
fn target_memory_backend(config: &Config) -> Result<Arc<dyn Memory>> {
    let binding = crate::openhuman::memory::binding::for_config(config)
        .map_err(|e| anyhow::anyhow!("bind memory for migration import: {e}"))?;
    if binding.class() == crate::core::subsystem::DriverClass::Null {
        let admitted = crate::openhuman::memory::binding::admit(&config.subsystems.memory).ok();
        let because = match (binding.fallback(), admitted) {
            (Some(fallback), _) => format!(
                "driver '{}' refused to bind: {}",
                fallback.configured_driver, fallback.reason
            ),
            // Admitted as the null driver: the user asked for no memory.
            (None, Some((id, crate::core::subsystem::DriverClass::Null)))
                if id == tinymemory_api::null::NULL_DRIVER_ID =>
            {
                "memory is disabled by configuration ([subsystems.memory] driver = \"null\")"
                    .to_string()
            }
            (None, Some((id, crate::core::subsystem::DriverClass::Null))) => format!(
                "driver '{id}' is configured with class \"null\" under \
                 [subsystems.memory.drivers.{id}], so every write is discarded"
            ),
            // Admitted as something that can hold data, bound to the null
            // provider anyway — the module substitution.
            (None, Some((id, _))) => format!(
                "driver '{id}' is configured, but this build has no memory module \
                 compiled in (the 'modules' feature is off), so nothing can be written"
            ),
            // `admit` refused: `build` records that as a fallback, so this is
            // unreachable. Named rather than merged into an arm that would
            // state a cause this branch cannot actually establish.
            (None, None) => format!(
                "driver '{}' bound the null provider and the reason was not recorded",
                binding.driver_id()
            ),
        };
        anyhow::bail!(
            "refusing to import memory into the null driver — {because}. \
             Nothing was imported; the source workspace is untouched."
        );
    }
    crate::openhuman::agent::experience::ops::DriverMemory::for_config(config)
        .map_err(|e| anyhow::anyhow!("bind memory for migration import: {e}"))
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

fn resolve_hermes_workspace(source: Option<PathBuf>) -> Result<PathBuf> {
    if let Some(path) = source {
        return Ok(path);
    }

    let Some(user_dirs) = UserDirs::new() else {
        bail!("Failed to determine user home directory");
    };

    #[cfg(windows)]
    {
        if let Some(local_app_data) = std::env::var_os("LOCALAPPDATA") {
            return Ok(PathBuf::from(local_app_data).join("hermes"));
        }
    }

    Ok(user_dirs.home_dir().join(".hermes"))
}

fn hermes_file_mappings() -> Vec<(&'static str, &'static str, MemoryCategory)> {
    vec![
        ("MEMORY.md", "hermes_memory", MemoryCategory::Core),
        (
            "USER.md",
            "hermes_user_profile",
            MemoryCategory::Custom("user_profile".to_string()),
        ),
        (
            "SOUL.md",
            "hermes_persona",
            MemoryCategory::Custom("persona".to_string()),
        ),
    ]
}

pub async fn migrate_hermes_memory(
    config: &Config,
    source_workspace: Option<PathBuf>,
    dry_run: bool,
) -> Result<MigrationReport> {
    let source_workspace = resolve_hermes_workspace(source_workspace)?;
    if !source_workspace.exists() {
        bail!(
            "Hermes workspace not found at {}. Provide a valid source workspace.",
            source_workspace.display()
        );
    }

    if paths_equal(&source_workspace, &config.workspace_dir) {
        bail!("Source workspace matches current OpenHuman workspace; refusing self-migration");
    }

    let mut stats = MigrationStats::default();
    let mut warnings = Vec::new();
    let mut entries = Vec::new();

    for (filename, key, category) in hermes_file_mappings() {
        let path = source_workspace.join(filename);
        if !path.exists() {
            warnings.push(format!(
                "{filename} not found in {}",
                source_workspace.display()
            ));
            continue;
        }
        let content = fs::read_to_string(&path)
            .with_context(|| format!("Failed to read {}", path.display()))?;
        if content.trim().is_empty() {
            warnings.push(format!("{filename} is empty, skipping"));
            continue;
        }
        entries.push(SourceEntry {
            key: key.to_string(),
            content: content.trim().to_string(),
            category,
        });
    }

    stats.from_markdown = entries.len();

    if entries.is_empty() {
        warnings.push(format!(
            "No importable memory found in {}",
            source_workspace.display()
        ));
        warnings.push("Checked for: MEMORY.md, USER.md, SOUL.md".to_string());
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

    for entry in entries {
        let mut key = entry.key;

        if let Some(existing) = memory.get("", &key).await? {
            if existing.content.trim() == entry.content.trim() {
                stats.skipped_unchanged += 1;
                continue;
            }
            let renamed = next_available_key(memory.as_ref(), &key).await?;
            key = renamed;
            stats.renamed_conflicts += 1;
        }

        memory
            .store("", &key, &entry.content, entry.category, None)
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
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
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

fn pick_optional_column_expr<'a>(columns: &'a [String], candidates: &[&'a str]) -> Option<&'a str> {
    candidates
        .iter()
        .find(|&candidate| columns.iter().any(|c| c.eq_ignore_ascii_case(candidate)))
        .map(|v| v as _)
}

async fn next_available_key(memory: &dyn Memory, key: &str) -> Result<String> {
    let mut idx = 1u32;
    loop {
        let candidate = format!("{key}_{idx}");
        if memory.get("", &candidate).await?.is_none() {
            return Ok(candidate);
        }
        idx += 1;
    }
}

#[cfg(test)]
#[path = "core_tests.rs"]
mod tests;
