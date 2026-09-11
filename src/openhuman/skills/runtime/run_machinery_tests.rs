use super::*;

#[tokio::test]
async fn workflow_profile_installs_memory_source_scope() {
    let mut profile = crate::openhuman::agent::profiles::store::built_in_default_profile();
    profile.memory_sources = Some(vec!["slack:#eng".into(), "github:openhuman".into()]);

    let visible = with_profile_memory_source_scope(Some(&profile), async {
        crate::openhuman::memory::source_scope::current_source_scope()
    })
    .await;

    assert_eq!(
        visible,
        Some(std::collections::HashSet::from([
            "slack:#eng".into(),
            "github:openhuman".into(),
        ]))
    );
    assert_eq!(
        crate::openhuman::memory::source_scope::current_source_scope(),
        None,
        "workflow scope must not leak after the run future finishes"
    );
}
