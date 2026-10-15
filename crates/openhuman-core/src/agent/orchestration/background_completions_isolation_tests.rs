use super::*;

pub(super) fn assert_session_cache_eviction() {
    // Other modules register completions without TEST_ENV_LOCK. Their inserts
    // legitimately evict older sessions from this process-wide bounded cache,
    // so exercise the exact eviction boundary in a process with no other tests.
    const CHILD: &str = "OPENHUMAN_SESSION_CACHE_EVICTION_TEST_CHILD";
    if std::env::var_os(CHILD).is_none() {
        let output = std::process::Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "agent::orchestration::background_completions::tests::the_session_cache_evicts_its_oldest_mapping_only",
                "--nocapture",
            ])
            .env(CHILD, "1")
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        return;
    }
    let _guard = test_guard();
    for i in 0..(SESSION_THREADS_CAP + 5) {
        note_session_thread(&format!("evict-sess-{i}"), &format!("evict-thread-{i}"));
    }
    assert_eq!(thread_for_session("evict-sess-0"), None, "oldest evicted");
    for survivor in [5, SESSION_THREADS_CAP / 2, SESSION_THREADS_CAP + 4] {
        assert_eq!(
            thread_for_session(&format!("evict-sess-{survivor}")).as_deref(),
            Some(format!("evict-thread-{survivor}").as_str()),
            "a surviving session still resolves to its own thread"
        );
    }
}
