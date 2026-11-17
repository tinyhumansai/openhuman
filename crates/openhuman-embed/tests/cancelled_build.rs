//! A cancelled `RuntimeBuilder::build` must release the process's runtime
//! slot. The desktop shell's startup timeout drops its server task mid-boot;
//! a leaked slot would make every later build fail with `AlreadyRunning`.
//! Its own test binary: the slot is process-wide.

use openhuman_embed::{RuntimeBuilder, RuntimeError};

#[test]
fn a_cancelled_build_releases_the_slot() {
    std::thread::Builder::new()
        .stack_size(16 * 1024 * 1024)
        .spawn(|| {
            tokio::runtime::Builder::new_multi_thread()
                .enable_all()
                .build()
                .expect("tokio runtime")
                .block_on(async {
                    // A zero deadline polls the build once, then drops it.
                    let first = tokio::time::timeout(
                        std::time::Duration::ZERO,
                        RuntimeBuilder::new().build(),
                    )
                    .await;
                    drop(first);

                    match RuntimeBuilder::new().build().await {
                        Ok(runtime) => drop(runtime),
                        Err(RuntimeError::AlreadyRunning) => {
                            panic!("a cancelled build left the runtime slot claimed")
                        }
                        Err(other) => panic!("second build failed: {other}"),
                    }
                });
        })
        .expect("test thread")
        .join()
        .expect("test thread should not panic");
}
