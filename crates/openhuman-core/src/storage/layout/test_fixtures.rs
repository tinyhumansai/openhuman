//! Fixture profiles for the layout tests. Everything is built inside a
//! `TempDir`; no path here ever resolves outside it.

use std::collections::BTreeMap;
use std::path::Path;

use sha2::Digest;
use tempfile::TempDir;

use super::{legacy, Target};
use crate::config::Config;
use crate::storage::{self, DocumentStoreExt, Precondition, Scope};

/// A v1 profile `users/u1` under a temporary root.
pub struct Fixture {
    pub dir: TempDir,
    pub target: Target,
    pub config: Config,
}

impl Fixture {
    pub fn root(&self) -> &Path {
        self.dir.path()
    }
    pub fn workspace(&self) -> std::path::PathBuf {
        self.target.workspace()
    }
}

pub fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A v1 profile with a populated database, user files, legacy state files and
/// two entries nothing classifies.
pub fn v1_profile() -> Fixture {
    let dir = TempDir::new().unwrap();
    let target = Target::new(dir.path(), "u1");
    let ws = target.workspace();
    std::fs::create_dir_all(&ws).unwrap();
    write(&target.profile_dir.join("config.toml"), "# fixture\n");
    // user files
    write(&ws.join("skills/s1/SKILL.md"), "# skill one\n");
    write(&ws.join("attachments/photo.jpg"), "not really a jpeg");
    write(&ws.join("artifacts/a1/plan.md"), "# plan\n");
    write(&ws.join("STYLE.md"), "be brief\n");
    write(&ws.join("SOUL.md"), "soul\n");
    // runtime files nobody imports: carried into files/
    write(&ws.join("state/costs.jsonl"), "{\"cost\":1}\n");
    write(&ws.join("memory/legacy_claims/c.json"), "{\"claim\":true}");
    // legacy state an importer reads, then the migration deletes
    write(&ws.join("memory/jobs.json"), "{\"jobs\":[1,2,3]}");
    write(
        &ws.join("memory/conversations/threads.jsonl"),
        "{\"op\":\"upsert\",\"thread_id\":\"t1\",\"title\":\"hello\",\"created_at\":\"2026-01-01T00:00:00Z\"}\n",
    );
    // unknown
    write(&ws.join("mystery/data.bin"), "???");
    write(&ws.join("notes.tmp"), "scratch");
    // the v1 database, with rows in a collection of our own
    let url = legacy::v1_url(&target);
    storage::block_on(async move {
        let backend = storage::open(&url).await?;
        let docs = backend.for_scope(&Scope::local())?.documents().clone();
        docs.ensure_collection(&tinystoragedrivers::CollectionSpec::new("layout_fixture"))
            .await?;
        for i in 0..3 {
            docs.put(
                "layout_fixture",
                &format!("d{i}"),
                serde_json::json!({ "n": i }),
                Precondition::Absent,
            )
            .await?;
        }
        Ok(())
    })
    .unwrap();
    let config = Config {
        workspace_dir: ws,
        config_path: target.profile_dir.join("config.toml"),
        ..Config::default()
    };
    Fixture {
        dir,
        target,
        config,
    }
}

/// Every file under `root` with the digest of its bytes (and `<dir>` markers
/// for directories), for before/after comparisons.
pub fn tree(root: &Path) -> BTreeMap<String, String> {
    fn walk(dir: &Path, root: &Path, out: &mut BTreeMap<String, String>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries {
            let path = entry.unwrap().path();
            let rel = path.strip_prefix(root).unwrap().display().to_string();
            if path.is_dir() {
                out.insert(rel, "<dir>".to_owned());
                walk(&path, root, out);
            } else {
                out.insert(
                    rel,
                    hex::encode(sha2::Sha256::digest(std::fs::read(&path).unwrap())),
                );
            }
        }
    }
    let mut out = BTreeMap::new();
    walk(root, root, &mut out);
    out
}

/// The `layout_fixture` documents of the database file `db`, by id.
pub fn fixture_docs(db: &Path) -> BTreeMap<String, serde_json::Value> {
    let url = format!("sqlite:{}", db.display());
    storage::block_on(async move {
        let backend = storage::open(&url).await?;
        let docs = backend.for_scope(&Scope::local())?.documents().clone();
        let found = docs
            .query_all("layout_fixture", &tinystoragedrivers::Query::all())
            .await?;
        Ok(found.into_iter().map(|d| (d.id, d.doc)).collect())
    })
    .unwrap()
}

/// The imported `memory/state/jobs` value of the database file `db`.
pub fn jobs_value(db: &Path) -> Option<serde_json::Value> {
    let url = format!("sqlite:{}", db.display());
    let scoped = storage::block_on(async move {
        let backend = storage::open(&url).await?;
        backend.for_scope(&Scope::local())
    })
    .unwrap();
    crate::storage::state_kv::StateKv::over(&scoped, "memory", "state")
        .get::<serde_json::Value>("jobs")
        .unwrap()
}
