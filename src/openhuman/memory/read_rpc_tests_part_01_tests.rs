use super::*;

/// The handler forwards the driver's flush outcome onto the wire unchanged.
///
/// The behaviour this test used to stage — an ingest producing a stale buffer,
/// the second flush deduplicating inside the window — is the *driver's* and
/// moved with the SQL to the conformance suite
/// (`flushing_twice_in_a_window_schedules_the_work_once`), where a real store
/// exists. What is the host's here is only the mapping: both fields pass
/// through, and the u64→u32 buffer count clamps rather than wraps.
#[tokio::test]
async fn flush_now_reports_the_drivers_outcome() {
    use crate::openhuman::memory::api::provider::types::FlushOutcome;

    let (_tmp, cfg) = test_config();
    crate::openhuman::memory::binding::install_for_test(
        &cfg.workspace_dir,
        &cfg.subsystems.memory,
        std::sync::Arc::new(
            crate::openhuman::memory::binding::FixedDiagnostics::new(
                Default::default(),
                Default::default(),
            )
            .flushing(FlushOutcome {
                enqueued: false,
                stale_buffers: u64::from(u32::MAX) + 7,
            }),
        ) as std::sync::Arc<dyn crate::openhuman::memory::api::provider::MemoryProvider>,
    );

    let resp = flush_now_rpc(&cfg).await.expect("flush_now").value;
    assert!(
        !resp.enqueued,
        "`enqueued: false` passes through — with a non-zero buffer count it \
         means \"already scheduled\", not \"nothing to do\""
    );
    assert_eq!(
        resp.stale_buffers,
        u32::MAX,
        "a count past the wire type's range clamps rather than wraps to a small lie"
    );
}
