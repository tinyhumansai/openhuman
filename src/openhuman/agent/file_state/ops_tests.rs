use super::*;
use std::time::Duration;

fn fresh_coordinator() -> Arc<FileStateCoordinator> {
    Arc::new(FileStateCoordinator::new())
}

#[test]
fn record_and_check_no_staleness() {
    let coord = fresh_coordinator();
    let path = PathBuf::from("/tmp/test/a.txt");
    coord.reads.write().insert(
        ("agent-a".to_string(), path.clone()),
        ReadStamp {
            mtime: SystemTime::now(),
            timestamp: Instant::now(),
            partial: false,
        },
    );
    let reads = coord.reads.read();
    let rs = reads.get(&("agent-a".to_string(), path.clone())).unwrap();
    assert!(!rs.partial);
    assert!(coord.writes.read().get(&path).is_none());
}

#[test]
fn detect_sibling_write_staleness() {
    let coord = fresh_coordinator();
    let path = PathBuf::from("/tmp/test/b.txt");
    let read_time = Instant::now();
    coord.reads.write().insert(
        ("agent-a".to_string(), path.clone()),
        ReadStamp {
            mtime: SystemTime::now(),
            timestamp: read_time,
            partial: false,
        },
    );
    std::thread::sleep(Duration::from_millis(5));
    coord.writes.write().insert(
        path.clone(),
        WriteStamp {
            writer: "agent-b".to_string(),
            timestamp: Instant::now(),
        },
    );
    let stale = coord.stale_reads_for_parent("agent-a");
    assert_eq!(stale, vec![path]);
}

#[test]
fn own_write_does_not_trigger_staleness() {
    let coord = fresh_coordinator();
    let path = PathBuf::from("/tmp/test/c.txt");
    let now = Instant::now();
    coord.reads.write().insert(
        ("agent-a".to_string(), path.clone()),
        ReadStamp {
            mtime: SystemTime::now(),
            timestamp: now,
            partial: false,
        },
    );
    std::thread::sleep(Duration::from_millis(5));
    coord.writes.write().insert(
        path.clone(),
        WriteStamp {
            writer: "agent-a".to_string(),
            timestamp: Instant::now(),
        },
    );
    let stale = coord.stale_reads_for_parent("agent-a");
    assert!(stale.is_empty());
}

#[test]
fn partial_read_detected() {
    let coord = fresh_coordinator();
    let path = PathBuf::from("/tmp/test/d.txt");
    coord.reads.write().insert(
        ("agent-a".to_string(), path.clone()),
        ReadStamp {
            mtime: SystemTime::now(),
            timestamp: Instant::now(),
            partial: true,
        },
    );
    let reads = coord.reads.read();
    let rs = reads.get(&("agent-a".to_string(), path.clone())).unwrap();
    assert!(rs.partial);
}

#[test]
fn parent_stale_files_detects_child_writes() {
    let coord = fresh_coordinator();
    let path = PathBuf::from("/tmp/test/e.txt");
    let parent_read_time = Instant::now();
    coord.reads.write().insert(
        ("parent".to_string(), path.clone()),
        ReadStamp {
            mtime: SystemTime::now(),
            timestamp: parent_read_time,
            partial: false,
        },
    );
    std::thread::sleep(Duration::from_millis(5));
    coord.writes.write().insert(
        path.clone(),
        WriteStamp {
            writer: "child-1".to_string(),
            timestamp: Instant::now(),
        },
    );
    let stale = coord.stale_reads_for_parent("parent");
    assert_eq!(stale, vec![path]);
}

#[test]
fn paths_written_by_collects_correctly() {
    let coord = fresh_coordinator();
    let p1 = PathBuf::from("/tmp/test/f1.txt");
    let p2 = PathBuf::from("/tmp/test/f2.txt");
    coord.writes.write().insert(
        p1.clone(),
        WriteStamp {
            writer: "child-1".to_string(),
            timestamp: Instant::now(),
        },
    );
    coord.writes.write().insert(
        p2.clone(),
        WriteStamp {
            writer: "child-2".to_string(),
            timestamp: Instant::now(),
        },
    );
    let result = coord.paths_written_by(&["child-1".to_string()]);
    assert_eq!(result.len(), 1);
    assert!(result.contains_key("child-1"));
    assert_eq!(result["child-1"], vec![p1]);
}

#[tokio::test]
async fn path_lock_serialises_access() {
    let coord = fresh_coordinator();
    let path = PathBuf::from("/tmp/test/lock.txt");
    let mutex = {
        let mut locks = coord.path_locks.write();
        locks
            .entry(path.clone())
            .or_insert_with(|| Arc::new(Mutex::new(())))
            .clone()
    };

    let guard = mutex.lock().await;
    assert!(mutex.try_lock().is_err());
    drop(guard);
    assert!(mutex.try_lock().is_ok());
}
