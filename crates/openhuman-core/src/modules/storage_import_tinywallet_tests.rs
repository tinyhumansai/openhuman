use super::*;
use crate::storage::{Scope, Storage};
use sha2::Digest;
use tinystoragedrivers_testkit::memory_storage;

fn scoped() -> Storage {
    memory_storage(&Scope::new("module:tinywallet@local").unwrap())
}

fn record(id: &str, status: &str) -> String {
    format!(
        r#"{{"id":"{id}","url":"https://api.example/x","asset":"0xusdc","amountAtomic":2500,"amountDisplay":"0.002500 USDC","recipient":"0xpayee","network":"eip155:8453","txSignature":null,"status":"{status}","timestamp":"2026-10-12T10:00:00Z"}}"#
    )
}

/// A workspace as the pre-storage ledger left it: a payment that changed status
/// appears twice under one id.
fn legacy_workspace() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(LEDGER_FILE);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(
        &file,
        format!(
            "{}\n{}\n\ngarbage line\n{}\n",
            record("p1", "pending"),
            record("p2", "settled"),
            record("p1", "settled")
        ),
    )
    .unwrap();
    dir
}

fn digest(path: &Path) -> Vec<u8> {
    sha2::Sha256::digest(std::fs::read(path).unwrap()).to_vec()
}

#[tokio::test]
async fn the_payment_history_moves_into_the_modules_stream_in_order() {
    let legacy = legacy_workspace();
    let scoped = scoped();
    let outcome = import_into(&DispatchCtx::host_default(), &scoped, legacy.path())
        .await
        .unwrap();
    assert_eq!(
        outcome,
        Outcome::Imported {
            payments: 3,
            skipped: 1
        }
    );
    let stored = scoped
        .streams()
        .read_window(PAYMENTS_STREAM, 0, 10)
        .await
        .unwrap();
    let seen: Vec<_> = stored
        .iter()
        .map(|entry| {
            (
                entry.value["id"].as_str().unwrap().to_owned(),
                entry.value["status"].as_str().unwrap().to_owned(),
            )
        })
        .collect();
    assert_eq!(
        seen,
        [("p1", "pending"), ("p2", "settled"), ("p1", "settled")]
            .map(|(id, status)| (id.to_owned(), status.to_owned()))
    );
    // The amounts survive field for field, which is what the budget sums.
    assert_eq!(stored[1].value["amountAtomic"], 2500);
}

#[tokio::test]
async fn the_legacy_ledger_is_left_untouched() {
    let legacy = legacy_workspace();
    let file = legacy.path().join(LEDGER_FILE);
    let before = digest(&file);
    import_into(&DispatchCtx::host_default(), &scoped(), legacy.path())
        .await
        .unwrap();
    assert_eq!(digest(&file), before);
}

#[tokio::test]
async fn a_second_run_does_not_repeat_the_history() {
    let legacy = legacy_workspace();
    let scoped = scoped();
    import_into(&DispatchCtx::host_default(), &scoped, legacy.path())
        .await
        .unwrap();
    assert_eq!(
        import_into(&DispatchCtx::host_default(), &scoped, legacy.path())
            .await
            .unwrap(),
        Outcome::AlreadyDone
    );
    assert_eq!(scoped.streams().len(PAYMENTS_STREAM).await.unwrap(), 3);
}

#[tokio::test]
async fn a_stream_the_module_already_wrote_is_not_preceded_by_old_records() {
    let legacy = legacy_workspace();
    let scoped = scoped();
    scoped
        .streams()
        .append(
            PAYMENTS_STREAM,
            serde_json::from_str(&record("live", "settled")).unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        import_into(&DispatchCtx::host_default(), &scoped, legacy.path())
            .await
            .unwrap(),
        Outcome::Imported {
            payments: 0,
            skipped: 1
        }
    );
    assert_eq!(scoped.streams().len(PAYMENTS_STREAM).await.unwrap(), 1);
}

#[tokio::test]
async fn a_workspace_without_a_ledger_writes_no_marker() {
    let empty = tempfile::tempdir().unwrap();
    let scoped = scoped();
    assert_eq!(
        import_into(&DispatchCtx::host_default(), &scoped, empty.path())
            .await
            .unwrap(),
        Outcome::NoSource
    );
    let legacy = legacy_workspace();
    assert!(matches!(
        import_into(&DispatchCtx::host_default(), &scoped, legacy.path())
            .await
            .unwrap(),
        Outcome::Imported { .. }
    ));
}

#[tokio::test]
async fn an_unreadable_ledger_is_retried_instead_of_marked_done() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join(LEDGER_FILE);
    std::fs::create_dir_all(file.parent().unwrap()).unwrap();
    std::fs::write(&file, [0xff, 0xfe]).unwrap();
    let scoped = scoped();
    assert!(
        import_into(&DispatchCtx::host_default(), &scoped, dir.path())
            .await
            .is_err()
    );
    assert_eq!(scoped.streams().len(PAYMENTS_STREAM).await.unwrap(), 0);

    std::fs::write(&file, format!("{}\n", record("p1", "settled"))).unwrap();
    assert_eq!(
        import_into(&DispatchCtx::host_default(), &scoped, dir.path())
            .await
            .unwrap(),
        Outcome::Imported {
            payments: 1,
            skipped: 0
        }
    );
}

/// Another process holding the import claim is not "no backend": the import
/// stays unsettled and the ledger gate refuses, instead of x402 opening an
/// empty ledger with a fresh allowance.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_live_foreign_claim_keeps_the_ledger_closed() {
    use crate::storage::import_state;
    use std::sync::{mpsc, Arc};

    let backend = crate::storage::test_support::memory_backend();
    let _installed =
        crate::storage::test_support::install_for_test("memory:x402-claim", Arc::clone(&backend));
    let ctx = DispatchCtx::host_default();
    let legacy = legacy_workspace();
    let scope = super::super::storage_host::current_module_scope(&ctx, MODULE_ID).unwrap();
    let repo = crate::storage::documents::Repo::on(
        &backend,
        &scope,
        "storage_import",
        import_state::collection_specs,
    )
    .unwrap();
    let step = import_state::step_id(
        MARKER_COLLECTION,
        &support::marker_id("x402", legacy.path()),
    );
    let (held_tx, held_rx) = mpsc::channel::<()>();
    let (release_tx, release_rx) = mpsc::channel::<()>();
    let holder = std::thread::spawn(move || {
        import_state::with_claim(&repo, import_state::HOST_OWNER, &step, |_| {
            held_tx.send(()).unwrap();
            release_rx.recv().unwrap();
            Ok(())
        })
        .unwrap()
    });
    held_rx.recv().unwrap();

    let outcome = import_legacy(&ctx, legacy.path()).await;
    let ledger_may_open = crate::modules::import_legacy_x402(&ctx, legacy.path()).await;
    release_tx.send(()).unwrap();
    holder.join().unwrap();

    assert!(
        outcome.is_err(),
        "a contended import must not settle as {outcome:?}"
    );
    assert!(
        !ledger_may_open,
        "the ledger must not open while another process holds the import"
    );
}
