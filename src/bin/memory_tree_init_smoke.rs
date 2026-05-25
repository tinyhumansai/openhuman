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
//! Arg is thread count (default 16, must be > 0). Higher = more contention.
//! Use `RUST_LOG=debug` to see per-worker results.
//!
//! Exit code: 0 if all threads Ok, 1 if any failed.

use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use openhuman_core::openhuman::config::Config;
use openhuman_core::openhuman::memory_store::chunks::store::with_connection;

fn main() -> ExitCode {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info"))
        .try_init()
        .ok();

    let workspace = match std::env::var("OPENHUMAN_WORKSPACE") {
        Ok(v) => PathBuf::from(v),
        Err(_) => {
            log::error!("OPENHUMAN_WORKSPACE must be set to a writable directory");
            return ExitCode::from(2);
        }
    };
    let raw = std::env::args().nth(1).unwrap_or_else(|| "16".into());
    let n: usize = match raw.parse() {
        Ok(v) if v > 0 => v,
        Ok(_) => {
            log::error!("thread count must be a positive integer (> 0), got {raw}");
            return ExitCode::from(2);
        }
        Err(e) => {
            log::error!("thread count must be a positive integer, got {raw:?}: {e}");
            return ExitCode::from(2);
        }
    };

    let mut cfg = Config::default();
    cfg.workspace_dir = workspace.clone();

    let db_path = workspace.join("memory_tree").join("chunks.db");
    let cold = !db_path.exists();
    log::info!(
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
                    log::debug!("worker {i:3} ok");
                }
                Err(e) => {
                    errors.fetch_add(1, Ordering::Relaxed);
                    log::error!("worker {i:3} FAILED: {e:#}");
                }
            })
        })
        .collect();

    for t in threads {
        t.join().expect("worker thread panicked");
    }

    let failed = errors.load(Ordering::Relaxed);
    let elapsed = start.elapsed();
    log::info!(
        "[smoke] done in {:?} — {}/{} ok, {} failed",
        elapsed,
        n - failed,
        n,
        failed
    );

    if failed > 0 {
        ExitCode::from(1)
    } else {
        ExitCode::SUCCESS
    }
}
