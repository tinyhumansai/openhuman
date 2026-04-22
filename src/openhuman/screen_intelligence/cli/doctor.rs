//! `openhuman screen-intelligence doctor` — diagnostic readiness check.

use anyhow::Result;

use super::{bootstrap_engine, init_quiet_logging, is_help, parse_opts};

pub(super) fn run_doctor(args: &[String]) -> Result<()> {
    if args.iter().any(|a| is_help(a)) {
        println!("Usage: openhuman screen-intelligence doctor [-v]");
        println!();
        println!("Check system readiness: permissions, platform support, vision config.");
        return Ok(());
    }

    let (opts, _) = parse_opts(args)?;
    init_quiet_logging(opts.verbose);

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?;

    rt.block_on(async {
        let _engine = bootstrap_engine(opts.verbose).await?;

        let doctor_json =
            crate::openhuman::screen_intelligence::rpc::accessibility_doctor_cli_json()
                .await
                .map_err(|e| anyhow::anyhow!("doctor check failed: {e}"))?;

        let summary = &doctor_json["result"]["summary"];
        let recommendations = &doctor_json["result"]["recommendations"];
        let permissions = &doctor_json["result"]["permissions"];

        eprintln!("  Screen Intelligence Doctor");
        eprintln!("  ──────────────────────────");
        eprintln!();

        let check = |ok: bool| if ok { "✓" } else { "✗" };

        let platform_ok = summary["platform_supported"].as_bool().unwrap_or(false);
        let screen_ok = summary["screen_capture_ready"].as_bool().unwrap_or(false);
        let control_ok = summary["accessibility_ready"].as_bool().unwrap_or(false);
        let input_ok = summary["input_monitoring_ready"].as_bool().unwrap_or(false);
        let overall_ok = summary["overall_ready"].as_bool().unwrap_or(false);

        eprintln!("  {} Platform supported", check(platform_ok));
        eprintln!("  {} Screen recording", check(screen_ok));
        eprintln!("  {} Accessibility automation", check(control_ok));
        eprintln!("  {} Input monitoring", check(input_ok));
        eprintln!();

        // Vision config check.
        let config = crate::openhuman::config::Config::load_or_init().await.ok();
        if let Some(ref cfg) = config {
            let si = &cfg.screen_intelligence;
            let la = &cfg.local_ai;
            eprintln!("  Config:");
            eprintln!("    enabled:           {}", si.enabled);
            eprintln!("    vision_enabled:    {}", si.vision_enabled);
            eprintln!("    use_vision_model:  {}", si.use_vision_model);
            eprintln!("    baseline_fps:      {}", si.baseline_fps);
            eprintln!("    keep_screenshots:  {}", si.keep_screenshots);
            eprintln!("    local_ai.enabled:  {}", la.enabled);
            eprintln!("    local_ai.provider: {}", la.provider);
            if si.vision_enabled && !la.enabled {
                eprintln!("    ⚠  Vision is enabled but local_ai.enabled=false — vision analysis will fail");
            }
        }

        eprintln!();
        if overall_ok {
            eprintln!("  ✓ Overall: READY");
        } else {
            eprintln!("  ✗ Overall: NOT READY");
            eprintln!();
            eprintln!("  Recommendations:");
            if let Some(recs) = recommendations.as_array() {
                for rec in recs {
                    if let Some(s) = rec.as_str() {
                        eprintln!("    • {s}");
                    }
                }
            }
        }
        eprintln!();

        // Machine-readable JSON output.
        println!(
            "{}",
            serde_json::to_string_pretty(&serde_json::json!({
                "summary": summary,
                "permissions": permissions,
                "config": config.as_ref().map(|c| serde_json::json!({
                    "enabled": c.screen_intelligence.enabled,
                    "vision_enabled": c.screen_intelligence.vision_enabled,
                    "use_vision_model": c.screen_intelligence.use_vision_model,
                    "baseline_fps": c.screen_intelligence.baseline_fps,
                    "keep_screenshots": c.screen_intelligence.keep_screenshots,
                    "local_ai_enabled": c.local_ai.enabled,
                    "local_ai_provider": c.local_ai.provider,
                })),
            }))
            .unwrap_or_default()
        );
        Ok(())
    })
}
