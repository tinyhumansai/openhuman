use super::*;

#[test]
fn the_tokio_runtime_is_multi_threaded_and_runs_futures() {
    let runtime = tokio_runtime().expect("runtime");
    assert_eq!(
        runtime.handle().runtime_flavor(),
        tokio::runtime::RuntimeFlavor::MultiThread
    );
    assert_eq!(runtime.block_on(async { 41 + 1 }), 42);
}

#[test]
fn the_worker_stack_is_sized_for_nested_turns() {
    // The default 2 MiB worker stack overflows on a delegated sub-agent turn.
    const { assert!(AGENT_WORKER_STACK_BYTES > 2 * 1024 * 1024) };
    const { assert!(MAX_BLOCKING_THREADS > 0) };
    // A deep frame on a worker must fit: 4 MiB of locals would abort on the
    // default stack.
    let runtime = tokio_runtime().expect("runtime");
    let sum = runtime.block_on(async {
        tokio::spawn(async {
            let buf = [1u8; 4 * 1024 * 1024];
            std::hint::black_box(&buf)
                .iter()
                .map(|b| *b as usize)
                .sum::<usize>()
        })
        .await
        .expect("join")
    });
    assert_eq!(sum, 4 * 1024 * 1024);
}

#[test]
fn log_directory_is_unset_without_a_file_logger() {
    // No test in this binary initialises file logging.
    assert!(log_directory().is_none());
}

#[test]
fn shutdown_file_guard_without_a_guard_reports_nothing_taken() {
    assert!(!shutdown_file_guard());
}
