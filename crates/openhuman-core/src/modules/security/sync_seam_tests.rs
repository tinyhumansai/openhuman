use super::run_on;
use std::time::Duration;

#[test]
fn missing_runtime_suppresses_without_returning_the_original() {
    let result = run_on(None, Duration::from_millis(20), async {
        Ok("secret".to_owned())
    });
    assert!(result.is_err());
}

#[test]
fn bridge_works_from_a_sync_thread_without_starting_another_runtime() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let handle = runtime.handle().clone();
    let result = run_on(Some(&handle), Duration::from_secs(1), async {
        Ok("module-redacted".to_owned())
    });
    assert_eq!(result.unwrap(), "module-redacted");
}

#[test]
fn same_current_thread_runtime_is_refused_to_avoid_deadlock() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let handle = runtime.handle().clone();
    let result = runtime.block_on(async {
        run_on(Some(&handle), Duration::from_millis(20), async {
            Ok("secret".to_owned())
        })
    });
    assert!(result.unwrap_err().contains("deadlock"));
}

#[test]
fn timed_out_module_answer_is_an_error() {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    let handle = runtime.handle().clone();
    let result = run_on(Some(&handle), Duration::from_millis(1), async {
        tokio::time::sleep(Duration::from_secs(1)).await;
        Ok("secret".to_owned())
    });
    assert!(result.unwrap_err().contains("timed out"));
}
