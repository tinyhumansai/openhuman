//! Obsidian vault defaults.
//!
//! When the memory_tree content root is first populated we drop a small
//! `.obsidian/` directory into it so a user opening the vault gets the
//! intended graph-view colour mapping (one colour per summary level) and
//! the front-matter type hints (`time_range_*` as `date`, `sealed_at` as
//! `datetime`) without any manual configuration.
//!
//! The bundled defaults live as static files under `obsidian_defaults/`
//! and are baked into the binary via `include_str!`. We only stage them
//! when the corresponding `.obsidian/<file>` doesn't already exist —
//! never overwrite a file the user has tweaked.
//!
//! Callers should invoke [`ensure_obsidian_defaults`] from any code path
//! that creates files under `content_root` (summary stage, raw write,
//! etc.). The function is idempotent and cheap on the steady-state path
//! (one `Path::exists()` per file).
//!
//! Failure mode: best-effort. A failed stage logs a warn and returns
//! `Ok(())` so seal/raw-write callers don't abort persistence over a
//! cosmetic vault default.

use std::path::Path;

use anyhow::Result;

const GRAPH_JSON: &str = include_str!("obsidian_defaults/graph.json");
const TYPES_JSON: &str = include_str!("obsidian_defaults/types.json");

/// Write the bundled `.obsidian/` defaults into `content_root` if they
/// aren't already there. Idempotent — never overwrites existing files.
pub fn ensure_obsidian_defaults(content_root: &Path) -> Result<()> {
    let obsidian_dir = content_root.join(".obsidian");
    if let Err(err) = std::fs::create_dir_all(&obsidian_dir) {
        log::warn!(
            "[content_store::obsidian] create .obsidian dir failed at {:?}: {err:#} — skipping defaults",
            obsidian_dir
        );
        return Ok(());
    }

    write_default_if_missing(&obsidian_dir, "graph.json", GRAPH_JSON);
    write_default_if_missing(&obsidian_dir, "types.json", TYPES_JSON);
    Ok(())
}

fn write_default_if_missing(obsidian_dir: &Path, name: &str, body: &str) {
    let target = obsidian_dir.join(name);
    if target.exists() {
        return;
    }
    match std::fs::write(&target, body) {
        Ok(()) => log::info!(
            "[content_store::obsidian] staged default {} at {}",
            name,
            target.display()
        ),
        Err(err) => log::warn!(
            "[content_store::obsidian] write default {} failed at {:?}: {err:#}",
            name,
            target
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn stages_defaults_into_fresh_root() {
        let tmp = TempDir::new().unwrap();
        ensure_obsidian_defaults(tmp.path()).unwrap();
        let graph = tmp.path().join(".obsidian").join("graph.json");
        let types = tmp.path().join(".obsidian").join("types.json");
        assert!(graph.exists(), "graph.json should be staged");
        assert!(types.exists(), "types.json should be staged");
        // Body must be the bundled content, not empty.
        let g = std::fs::read_to_string(&graph).unwrap();
        assert!(g.contains("colorGroups"), "graph.json missing colorGroups");
    }

    #[test]
    fn does_not_overwrite_existing_file() {
        let tmp = TempDir::new().unwrap();
        let obs = tmp.path().join(".obsidian");
        std::fs::create_dir_all(&obs).unwrap();
        let graph = obs.join("graph.json");
        std::fs::write(&graph, r#"{"user":"custom"}"#).unwrap();

        ensure_obsidian_defaults(tmp.path()).unwrap();

        let body = std::fs::read_to_string(&graph).unwrap();
        assert_eq!(
            body, r#"{"user":"custom"}"#,
            "user-customised graph.json must not be clobbered"
        );
    }

    #[test]
    fn idempotent_second_call_is_no_op() {
        let tmp = TempDir::new().unwrap();
        ensure_obsidian_defaults(tmp.path()).unwrap();
        ensure_obsidian_defaults(tmp.path()).unwrap();
        // Second call must succeed without panicking and must not have
        // duplicated or grown the file.
        let g = std::fs::read_to_string(tmp.path().join(".obsidian/graph.json")).unwrap();
        assert!(g.contains("colorGroups"));
    }
}
