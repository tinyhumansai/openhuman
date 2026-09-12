//! On-disk artefact writer for `dump_all_agent_prompts`.
//!
//! Owns the byte-stable file layout the CLI previously inlined:
//!
//! * `{idx}_{agent}[_{toolkit}].md`       — raw system prompt bytes
//! * `{idx}_{agent}[_{toolkit}].meta.txt` — key/value metadata sidecar
//! * `SUMMARY.txt`                        — one fixed-width row per dump
//!
//! Format is exercised by the golden test in this file; any field
//! reorder or width change is a breaking artefact change and must land
//! with a test update.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

use super::DumpedPrompt;

/// What [`write_prompt_dumps`] wrote, in the order it wrote it.
#[derive(Debug, Clone)]
pub struct DumpWriteSummary {
    /// Paths to the per-dump `.md` files, in the same order as the
    /// input slice.
    pub prompt_paths: Vec<PathBuf>,
    /// Path to the `SUMMARY.txt` file.
    pub summary_path: PathBuf,
}

/// Write a batch of [`DumpedPrompt`]s into `dir` using the stable
/// on-disk layout the CLI depends on. `dir` is created if it does
/// not yet exist; the call fails only on a permission or I/O error.
///
/// Emits `[dump-all] …` progress lines on stderr so the CLI surface
/// matches pre-extraction behaviour byte-for-byte.
pub fn write_prompt_dumps(dir: &Path, dumps: &[DumpedPrompt]) -> Result<DumpWriteSummary> {
    std::fs::create_dir_all(dir)
        .with_context(|| format!("creating output dir {}", dir.display()))?;

    let mut prompt_paths = Vec::with_capacity(dumps.len());
    let mut summary = String::new();

    for (idx, dumped) in dumps.iter().enumerate() {
        let stem = stem_for(idx, dumped);
        let prompt_path = dir.join(format!("{stem}.md"));
        let meta_path = dir.join(format!("{stem}.meta.txt"));

        std::fs::write(&prompt_path, &dumped.text)
            .with_context(|| format!("writing {}", prompt_path.display()))?;
        std::fs::write(&meta_path, render_meta(dumped))
            .with_context(|| format!("writing {}", meta_path.display()))?;

        // The tool schemas are sent alongside the prompt on every turn, so a
        // dump that omits them under-reports the fixed per-turn cost.
        let tools_path = dir.join(format!("{stem}.tools.json"));
        std::fs::write(
            &tools_path,
            serde_json::to_vec_pretty(&dumped.tool_specs)
                .with_context(|| format!("serialising tool specs for {stem}"))?,
        )
        .with_context(|| format!("writing {}", tools_path.display()))?;

        let label = label_for(dumped);
        let _ = writeln!(
            summary,
            "{:<32} tools={:<4} skill={:<4}",
            label,
            dumped.tool_names.len(),
            dumped.skill_tool_count
        );
        eprintln!("[dump-all] {label:<32} → {}", prompt_path.display());

        prompt_paths.push(prompt_path);
    }

    let summary_path = dir.join("SUMMARY.txt");
    std::fs::write(&summary_path, &summary)
        .with_context(|| format!("writing {}", summary_path.display()))?;
    eprintln!("[dump-all] wrote summary → {}", summary_path.display());

    Ok(DumpWriteSummary {
        prompt_paths,
        summary_path,
    })
}

fn stem_for(idx: usize, dumped: &DumpedPrompt) -> String {
    let safe_agent = sanitise_filename_component(&dumped.agent_id);
    match &dumped.toolkit {
        Some(tk) => format!(
            "{}_{}_{}",
            idx + 1,
            safe_agent,
            sanitise_filename_component(tk)
        ),
        None => format!("{}_{}", idx + 1, safe_agent),
    }
}

fn label_for(dumped: &DumpedPrompt) -> String {
    match &dumped.toolkit {
        Some(tk) => format!("{}@{}", dumped.agent_id, tk),
        None => dumped.agent_id.clone(),
    }
}

fn render_meta(dumped: &DumpedPrompt) -> String {
    let mut meta = String::new();
    let _ = writeln!(meta, "agent:          {}", dumped.agent_id);
    if let Some(tk) = &dumped.toolkit {
        let _ = writeln!(meta, "toolkit:        {tk}");
    }
    let _ = writeln!(meta, "mode:           {}", dumped.mode);
    let _ = writeln!(meta, "model:          {}", dumped.model);
    let _ = writeln!(meta, "workspace:      {}", dumped.workspace_dir.display());
    let _ = writeln!(meta, "tool_count:     {}", dumped.tool_names.len());
    let _ = writeln!(meta, "skill_tools:    {}", dumped.skill_tool_count);
    meta
}

fn sanitise_filename_component(value: &str) -> String {
    value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '.' || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "dump_writer_tests.rs"]
mod tests;
