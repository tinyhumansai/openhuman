use super::*;
#[tokio::test]
async fn continuation_preferences_survive_reload_and_follow_user_changes() {
    let root = tempfile::tempdir().unwrap();
    assert_eq!(load(root.path(), "thread").await.unwrap(), None);
    let selected = Preferences {
        model: Some("provider/chosen".into()),
        reasoning: Some("xhigh".into()),
        temperature: Some(0.2),
    };
    save(root.path(), "thread", &selected).await.unwrap();
    assert_eq!(load(root.path(), "thread").await.unwrap(), Some(selected));
    assert_eq!(load(root.path(), "other").await.unwrap(), None);
    save(root.path(), "thread", &Preferences::default())
        .await
        .unwrap();
    assert_eq!(
        load(root.path(), "thread").await.unwrap(),
        Some(Preferences::default())
    );
}
