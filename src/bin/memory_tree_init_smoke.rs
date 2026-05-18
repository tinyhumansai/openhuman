//! Manual stress smoke for the memory_tree schema-init race fix.
//!
//! Spins N concurrent threads racing into `memory::tree::store::with_connection`
//! against a shared workspace. Pre-fix (without the mutex-gated init guard),
//! cold-start runs would surface SQLite codes 14 (CANTOPEN), 1546
//! (IOERR_TRUNCATE), or 4874 (IOERR_SHMMAP) on some threads. Post-fix,
//! all N threads must return Ok.
//!
//! # Usage
//!
//! ```sh
//! # Fresh workspace (forces cold-start path)
//! rm -rf /tmp/mt-smoke
//! OPENHUMAN_WORKSPACE=/tmp/mt-smoke \
//!   cargo run --bin memory-tree-init-smoke -- 32
//!
//! # Re-run against warm DB (should also be Ok; exercises fast path)
//! OPENHUMAN_WORKSPACE=/tmp/mt-smoke \
//!   cargo run --bin memory-tree-init-smoke -- 32
//! ```
//!
//! Arg is thread count (default 16). Higher = more contention.
//!
//! Exit code: 0 if all threads Ok, 1 if any failed.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory::tree::store::with_connection;

fn main() {
    let workspace = std::env::var("OPENHUMAN_WORKSPACE")
        .map(PathBuf::from)
        .expect("set OPENHUMAN_WORKSPACE to a writable directory");
    let n: usize = std::env::args()
        .nth(1)
        .as_deref()
        .unwrap_or("16")
        .parse()
        .expect("thread count must be a positive integer");

    let mut cfg = Config::default();
    cfg.workspace_dir = workspace.clone();

    let db_path = workspace.join("memory_tree").join("chunks.db");
    let cold = !db_path.exists();
    eprintln!(
        "[smoke] workspace={} cold_start={} threads={}",
        workspace.display(),
        cold,
        n
    );

    let errors = Arc::new(AtomicUsize::new(0));
    let start = std::time::Instant::now();

    let threads: Vec<_> = (0..n)
        .map(|i| {
            let cfg = cfg.clone();
            let errors = errors.clone();
            std::thread::spawn(move || match with_connection(&cfg, |_| Ok(())) {
                Ok(_) => {
                    println!("worker {i:3} ok");
                }
                Err(e) => {
                    errors.fetch_add(1, Ordering::Relaxed);
                    eprintln!("worker {i:3} FAILED: {e:#}");
                }
            })
        })
        .collect();

    for t in threads {
        t.join().expect("worker thread panicked");
    }

    let failed = errors.load(Ordering::Relaxed);
    let elapsed = start.elapsed();
    eprintln!(
        "[smoke] done in {:?} — {}/{} ok, {} failed",
        elapsed,
        n - failed,
        n,
        failed
    );

    if failed > 0 {
        std::process::exit(1);
    }
}
