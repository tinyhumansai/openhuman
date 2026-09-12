use super::*;

#[test]
fn defaults_to_not_denied() {
    let _g = test_lock();
    clear();
    assert!(!system_events_denied());
}

#[test]
fn mark_then_observe() {
    let _g = test_lock();
    clear();
    assert!(!system_events_denied());
    mark_system_events_denied();
    assert!(system_events_denied());
    clear();
    assert!(!system_events_denied());
}

#[test]
fn idempotent_mark_and_clear() {
    let _g = test_lock();
    clear();
    mark_system_events_denied();
    mark_system_events_denied();
    assert!(system_events_denied());
    clear();
    clear();
    assert!(!system_events_denied());
}

#[test]
fn concurrent_mark_and_read() {
    let _g = test_lock();
    clear();
    let producers: Vec<_> = (0..8)
        .map(|_| std::thread::spawn(mark_system_events_denied))
        .collect();
    let readers: Vec<_> = (0..8)
        .map(|_| std::thread::spawn(|| system_events_denied()))
        .collect();
    for h in producers {
        h.join().unwrap();
    }
    for h in readers {
        // Read may race the marks — only the post-join state is
        // load-bearing for correctness.
        let _ = h.join().unwrap();
    }
    assert!(system_events_denied());
    clear();
}
