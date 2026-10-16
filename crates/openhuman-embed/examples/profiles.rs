//! Title: SaaS profiles isolate conversation history
//! Summary: SaaS profiles isolate conversation history.
//! Run: offline with loopback stubs; no live path.
//! Feature: channels

mod support;
fn main() -> anyhow::Result<()> {
    let root = tempfile::tempdir()?;
    let config = openhuman_embed::SaasConfig::new(root.path());
    let workspace = config.operator_config().workspace_dir;
    support::run_with_workspace(run(config), &workspace)
}
async fn run(config: openhuman_embed::SaasConfig) -> anyhow::Result<()> {
    let provider = support::echo_inference().await;
    support::PointedInference::install(&provider.uri());
    // ANCHOR: profiles
    let profiles = openhuman_embed::ProfileRuntime::build(config).await?;
    let mut handles = Vec::new();
    for (user, text) in [("alice", "Alice private note"), ("bob", "Bob private note")] {
        let profile = profiles.provision(user).await?;
        profiles
            .set_credential(
                &profile.profile_id,
                openhuman_embed::profiles::ProfileCredentialKind::ApiKey,
                &format!("{user}-fixture"),
            )
            .await?;
        let handle = profiles.open(user).await?;
        let reply = handle.chat("shared-thread", text).await?;
        assert!(reply.text.contains(text), "{}", reply.text);
        handles.push(handle);
    }
    assert_ne!(handles[0].workspace_dir(), handles[1].workspace_dir());
    for (index, own, other) in [
        (0, "Alice private note", "Bob private note"),
        (1, "Bob private note", "Alice private note"),
    ] {
        let messages = handles[index].messages("shared-thread").await?;
        let text = serde_json::to_string(&messages)?;
        assert!(text.contains(own));
        assert!(!text.contains(other));
    }
    drop(handles);
    profiles.shutdown().await;
    println!("two profiles keep same-thread conversations separate");
    // ANCHOR_END: profiles
    support::passed("profiles");
    Ok(())
}
