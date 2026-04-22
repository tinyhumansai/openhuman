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
/// on-disk layout the CLI depends on. Callers are expected to have
/// already created `dir` (or let this helper propagate the error).
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

pub(crate) fn sanitise_filename_component(value: &str) -> String {
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
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn sample_dump(agent: &str, toolkit: Option<&str>, tool_names: &[&str]) -> DumpedPrompt {
        DumpedPrompt {
            agent_id: agent.to_string(),
            toolkit: toolkit.map(|s| s.to_string()),
            mode: "session",
            model: "claude-opus-4-7".to_string(),
            workspace_dir: PathBuf::from("/tmp/ws"),
            text: format!("# prompt for {agent}\nbody\n"),
            tool_names: tool_names.iter().map(|s| s.to_string()).collect(),
            skill_tool_count: 1,
        }
    }

    #[test]
    fn golden_layout_matches_cli_format() {
        let dir = tempfile::tempdir().unwrap();
        let dumps = vec![
            sample_dump("orchestrator", None, &["a", "b", "c"]),
            sample_dump("integrations_agent", Some("gmail"), &["send", "search"]),
        ];

        let out = write_prompt_dumps(dir.path(), &dumps).unwrap();

        // File set exactly as expected.
        assert_eq!(out.prompt_paths.len(), 2);
        assert_eq!(out.prompt_paths[0], dir.path().join("1_orchestrator.md"));
        assert_eq!(
            out.prompt_paths[1],
            dir.path().join("2_integrations_agent_gmail.md")
        );
        assert_eq!(out.summary_path, dir.path().join("SUMMARY.txt"));

        // Prompt body is raw bytes.
        let body = std::fs::read_to_string(&out.prompt_paths[0]).unwrap();
        assert_eq!(body, "# prompt for orchestrator\nbody\n");

        // Meta sidecar: exact byte format, toolkit-less variant.
        let meta0 = std::fs::read_to_string(dir.path().join("1_orchestrator.meta.txt")).unwrap();
        let expected_meta0 = "\
agent:          orchestrator
mode:           session
model:          claude-opus-4-7
workspace:      /tmp/ws
tool_count:     3
skill_tools:    1
";
        assert_eq!(meta0, expected_meta0);

        // Meta sidecar: toolkit variant inserts `toolkit:` after `agent:`.
        let meta1 = std::fs::read_to_string(dir.path().join("2_integrations_agent_gmail.meta.txt"))
            .unwrap();
        let expected_meta1 = "\
agent:          integrations_agent
toolkit:        gmail
mode:           session
model:          claude-opus-4-7
workspace:      /tmp/ws
tool_count:     2
skill_tools:    1
";
        assert_eq!(meta1, expected_meta1);

        // SUMMARY.txt: one fixed-width row per dump.
        let summary = std::fs::read_to_string(&out.summary_path).unwrap();
        // Note: `{:<4}` pads the numeric fields, so rows carry three
        // trailing spaces. Preserved byte-for-byte from the pre-split
        // CLI implementation — any change here is an artefact-format
        // break.
        let expected_summary = "\
orchestrator                     tools=3    skill=1   \n\
integrations_agent@gmail         tools=2    skill=1   \n";
        assert_eq!(summary, expected_summary);
    }

    #[test]
    fn sanitises_filename_components() {
        assert_eq!(sanitise_filename_component("gmail"), "gmail");
        assert_eq!(sanitise_filename_component("a/b c"), "a_b_c");
        assert_eq!(sanitise_filename_component("..-_ok"), "..-_ok");
        assert_eq!(sanitise_filename_component("weird:name*"), "weird_name_");
    }
}
