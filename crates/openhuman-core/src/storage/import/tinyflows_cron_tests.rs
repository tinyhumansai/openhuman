use super::*;
use crate::cron::client::testing as cron_store;
use crate::storage::MemoryStorage;

fn copy_tree(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let target = to.join(entry.file_name());
        if entry.path().is_dir() {
            copy_tree(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), target).unwrap();
        }
    }
}

/// The cron importer rides the `schedule` reader alone, so `run_with` keeps the
/// legacy cron jobs in a build without `flows` too (it used to log and skip).
#[test]
fn run_with_imports_legacy_cron_jobs_whatever_the_flows_gate() {
    let dir = tempfile::TempDir::new().unwrap();
    let fixtures = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("src/storage/import/fixtures/tinyflows/workspace");
    copy_tree(&fixtures, dir.path());
    let config = Config {
        workspace_dir: dir.path().to_path_buf(),
        ..Config::default()
    };
    let backend: Arc<dyn StorageBackend> = Arc::new(MemoryStorage::new());

    let reports = run_with(&backend, &config);
    let cron = reports
        .iter()
        .find(|report| report.domain == CRON_DOMAIN)
        .expect("a cron report");
    match cron.outcome.as_ref().unwrap() {
        Outcome::Imported { written, .. } => assert!(*written > 0),
        other => panic!("expected the cron jobs to import, got {other:?}"),
    }
    let jobs = cron_store::list_jobs(&backend, &Scope::local()).unwrap();
    assert!(!jobs.is_empty());
}
