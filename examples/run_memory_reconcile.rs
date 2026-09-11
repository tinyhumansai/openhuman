//! Kick the sources coverage reconcile (report + execute) and stay alive
//! while the spawned summarise+ingest work drains. Same config-resolution
//! rules as run_memory_doctor: run with NO OPENHUMAN_WORKSPACE override.

#[tokio::main(flavor = "multi_thread")]
async fn main() -> anyhow::Result<()> {
    let mut config = openhuman_core::openhuman::config::Config::load_or_init()
        .await
        .unwrap_or_default();
    config.apply_env_overrides();
    eprintln!("config_path={}", config.config_path.display());

    openhuman_core::openhuman::memory::host::install_memory_event_sink();
    #[cfg(feature = "modules")]
    openhuman_core::openhuman::modules::memory::set_modules_policy(std::sync::Arc::new(
        config.clone(),
    ));

    let request = openhuman_core::openhuman::memory::sources::rpc::ReconcileRequest {
        source_id: None,
        execute: true,
    };
    let outcome = openhuman_core::openhuman::memory::sources::rpc::reconcile_rpc(request)
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
    println!("{}", serde_json::to_string_pretty(&outcome.value)?);

    // The execute arm spawns background summarise+ingest; exiting now would
    // kill it. Hold the process until the pending count stops moving.
    let mut last = u64::MAX;
    for i in 0..60 {
        tokio::time::sleep(std::time::Duration::from_secs(15)).await;
        let report = openhuman_core::openhuman::memory::sources::rpc::reconcile_rpc(
            openhuman_core::openhuman::memory::sources::rpc::ReconcileRequest {
                source_id: None,
                execute: false,
            },
        )
        .await
        .map_err(|e| anyhow::anyhow!(e))?;
        let pending: u64 = serde_json::to_value(&report.value)?
            .get("scopes")
            .and_then(|s| s.as_array())
            .map(|rows| {
                rows.iter()
                    .filter_map(|r| r.get("pending").and_then(|p| p.as_u64()))
                    .sum()
            })
            .unwrap_or(0);
        eprintln!("t+{}s pending={pending}", (i + 1) * 15);
        if pending == 0 || pending == last {
            if pending == 0 {
                break;
            }
            // Two identical non-zero readings in a row: still draining or
            // stalled — keep waiting either way, the cap bounds us.
        }
        last = pending;
    }
    Ok(())
}
