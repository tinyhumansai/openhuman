//! Title: Tenant scoped memory facade with an in-memory engine
//! Summary: Tenant scoped memory facade with an in-memory engine.
//! Run: offline with loopback stubs; no live path.
//! Feature: default

mod support;
use openhuman_embed::{Runtime, Workspace};
fn main() -> anyhow::Result<()> {
    support::run(run())
}
async fn run() -> anyhow::Result<()> {
    let backend = support::stub_backend().await;
    let engine =
        std::sync::Arc::new(openhuman_embed::memory::api::conformance::ReferenceEngine::new());
    let runtime = Runtime::builder()
        .workspace(Workspace::Ephemeral)
        .config(support::offline_config())
        .backend_url(backend.uri())
        .memory_engine(engine)
        .build()
        .await?;
    // ANCHOR: memory
    let acme = runtime.memory("team:acme")?;
    let other = runtime.memory("team:other")?;
    let params = serde_json::from_value(serde_json::json!({"text":"Acme ships on Fridays"}))?;
    let learned = acme.learn(params).await?;
    let hits = acme.get(vec![learned.id.clone()]).await?;
    assert_eq!(hits.len(), 1);
    assert_eq!(hits[0].meta.namespace.to_string(), "team:acme");
    assert!(other.get(vec![learned.id.clone()]).await?.is_empty());
    assert_eq!(other.forget(vec![learned.id.clone()]).await?.forgotten, 0);
    assert_eq!(acme.forget(vec![learned.id]).await?.forgotten, 1);
    println!("memory learning, tenant isolation and deletion verified");
    // ANCHOR_END: memory
    support::passed("memory");
    Ok(())
}
