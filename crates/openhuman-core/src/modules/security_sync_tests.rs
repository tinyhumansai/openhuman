use super::*;
use std::sync::Arc;

const SHORT: Duration = Duration::from_millis(200);

fn module_runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .unwrap()
}

async fn ok(value: u32) -> Result<u32, String> {
    Ok(value)
}

#[test]
fn answers_from_outside_any_runtime() {
    let rt = module_runtime();
    let gate = AtomicUsize::new(0);
    assert_eq!(run_on(&gate, 4, rt.handle(), SHORT, ok(7)), Ok(7));
    assert_eq!(gate.load(Ordering::SeqCst), 0);
}

#[test]
fn answers_from_a_worker_thread_of_the_module_runtime() {
    let rt = module_runtime();
    let handle = rt.handle().clone();
    let gate = Arc::new(AtomicUsize::new(0));
    let inner = Arc::clone(&gate);
    let out = rt.block_on(async move {
        tokio::spawn(async move { run_on(&inner, 4, &handle, SHORT, ok(9)) })
            .await
            .unwrap()
    });
    assert_eq!(out, Ok(9));
    assert_eq!(gate.load(Ordering::SeqCst), 0);
}

#[test]
fn current_thread_caller_on_the_module_runtime_is_refused_not_deadlocked() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let handle = rt.handle().clone();
    let gate = AtomicUsize::new(0);
    let out = rt.block_on(async { run_on(&gate, 4, &handle, SHORT, ok(1)) });
    assert_eq!(out, Err(SeamError::WouldDeadlock));
    assert_eq!(gate.load(Ordering::SeqCst), 0);
}

#[test]
fn current_thread_caller_on_another_runtime_is_served() {
    let module = module_runtime();
    let caller = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    let gate = AtomicUsize::new(0);
    let out = caller.block_on(async { run_on(&gate, 4, module.handle(), SHORT, ok(3)) });
    assert_eq!(out, Ok(3));
}

#[test]
fn a_slow_module_times_out_within_the_bound() {
    let rt = module_runtime();
    let gate = AtomicUsize::new(0);
    let started = std::time::Instant::now();
    let out: Result<u32, _> = run_on(&gate, 4, rt.handle(), Duration::from_millis(50), async {
        tokio::time::sleep(Duration::from_secs(30)).await;
        Ok(1)
    });
    assert_eq!(out, Err(SeamError::TimedOut));
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(gate.load(Ordering::SeqCst), 0);
}

#[test]
fn module_error_and_panic_fail_closed() {
    let rt = module_runtime();
    let gate = AtomicUsize::new(0);
    let err: Result<u32, _> = run_on(&gate, 4, rt.handle(), SHORT, async {
        Err("boom".to_string())
    });
    assert_eq!(err, Err(SeamError::Failed("boom".into())));
    let panicked: Result<u32, _> = run_on(&gate, 4, rt.handle(), SHORT, async {
        panic!("module task panicked");
    });
    assert!(
        matches!(panicked, Err(SeamError::Failed(_))),
        "{panicked:?}"
    );
    assert_eq!(gate.load(Ordering::SeqCst), 0);
}

#[test]
fn missing_module_runtime_is_unavailable() {
    let slot = OnceLock::new();
    let gate = AtomicUsize::new(0);
    let out = run_slot(&slot, &gate, 4, SHORT, ok(1));
    assert_eq!(out, Err(SeamError::Unavailable));
    assert_eq!(gate.load(Ordering::SeqCst), 0);
    let text: String = SeamError::Unavailable.into();
    assert!(text.contains("unavailable"));
}

#[test]
fn concurrent_callers_are_all_answered() {
    let rt = module_runtime();
    let handle = rt.handle().clone();
    let gate = Arc::new(AtomicUsize::new(0));
    let threads: Vec<_> = (0..16u32)
        .map(|n| {
            let (handle, gate) = (handle.clone(), Arc::clone(&gate));
            std::thread::spawn(move || run_on(&gate, 64, &handle, SHORT, ok(n)))
        })
        .collect();
    for (n, thread) in threads.into_iter().enumerate() {
        assert_eq!(thread.join().unwrap(), Ok(n as u32));
    }
    assert_eq!(gate.load(Ordering::SeqCst), 0);
}

#[test]
fn callers_over_the_limit_are_refused_and_permits_are_returned() {
    let rt = module_runtime();
    let handle = rt.handle().clone();
    let gate = Arc::new(AtomicUsize::new(0));
    let threads: Vec<_> = (0..6)
        .map(|_| {
            let (handle, gate) = (handle.clone(), Arc::clone(&gate));
            std::thread::spawn(move || {
                run_on(&gate, 2, &handle, Duration::from_secs(2), async {
                    tokio::time::sleep(Duration::from_millis(400)).await;
                    Ok(1u32)
                })
            })
        })
        .collect();
    let results: Vec<_> = threads.into_iter().map(|t| t.join().unwrap()).collect();
    assert!(results.contains(&Err(SeamError::Saturated)));
    assert!(results.contains(&Ok(1)));
    assert_eq!(gate.load(Ordering::SeqCst), 0);
}

#[test]
fn the_global_seam_is_unavailable_until_armed_outside_a_runtime() {
    assert!(!arm_current());
}

/// The module bus's own runtime outlives any caller's, so a seam armed with it
/// answers from a plain thread no matter which runtime first called the module.
/// Uses a private bus runtime and slot, not the process-global ones.
#[test]
fn a_seam_armed_with_the_module_bus_runtime_answers_from_a_plain_thread() {
    use tinybus::module::{ModuleRuntime, RuntimeNames};

    let names = RuntimeNames {
        thread: "seam-test-bus".to_string(),
        worker: "seam-test-worker".to_string(),
    };
    let runtime = ModuleRuntime::start(&names, None).expect("private module runtime starts");
    let slot = OnceLock::new();
    let gate = AtomicUsize::new(0);
    assert_eq!(
        run_slot(&slot, &gate, 4, SHORT, ok(1)),
        Err(SeamError::Unavailable)
    );
    assert!(arm_slot_from(&slot, &runtime));
    assert!(arm_slot_from(&slot, &runtime), "arming is idempotent");
    assert_eq!(
        run_slot(&slot, &gate, 4, Duration::from_secs(5), ok(7)),
        Ok(7)
    );
    assert_eq!(gate.load(Ordering::SeqCst), 0);
}
