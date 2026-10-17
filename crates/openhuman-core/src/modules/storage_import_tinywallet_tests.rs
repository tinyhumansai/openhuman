use super::*;
use sha2::Digest;
use tinystoragedrivers::{MemoryStorage, Scope, StorageBackend};

fn scoped() -> ScopedStorage {
    MemoryStorage::new()
        .for_scope(&Scope::new("module:tinywallet@local").unwrap())
        .unwrap()
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
