//! Title: Stable capability metadata before and after boot
//! Summary: Stable capability metadata before and after boot.
//! Run: offline with loopback stubs; no live path.
//! Feature: default

mod support;
use openhuman_embed::{Runtime, Workspace};

fn main() -> anyhow::Result<()> {
    if std::env::args().any(|argument| argument == "--json") {
        println!(
            "{}",
            serde_json::to_string_pretty(&openhuman_embed::RuntimeBuilder::library().describe())?
        );
        return Ok(());
    }
    support::run(run())
}

async fn run() -> anyhow::Result<()> {
    let backend = support::stub_backend().await;
    let provider = support::provider("hello from the stub").await;
    let runtime = Runtime::builder()
        .config(support::offline_config())
        .workspace(Workspace::Ephemeral)
        .backend_url(backend.uri())
        .provider(support::route(&provider, "fixture"))
        .build()
        .await?;
    // ANCHOR: capability_report
    let info = runtime.capabilities();
    assert_eq!(info.schema_version, 1);
    assert!(info.defaults.routed_provider);
    let serialized = serde_json::to_string(&info)?;
    assert!(!serialized.contains("sk-test"));
    assert!(!serialized.contains(&provider.uri()));
    assert!(!serialized.contains(&runtime.workspace_dir().display().to_string()));
    println!("{}", serde_json::to_string_pretty(&info)?);
    // ANCHOR_END: capability_report
    support::passed("capability_report");
    Ok(())
}
