use super::*;

#[tokio::test]
async fn queued_messages_are_taken_once() {
    publish(Some("s1".into()), "run the tests".into()).await;
    assert_eq!(take(Some("s1")).await, vec!["run the tests".to_string()]);
    assert!(take(Some("s1")).await.is_empty());
}

#[tokio::test]
async fn sessionless_followups_land_in_their_own_bucket() {
    publish(None, "no session".into()).await;
    assert!(take(Some("s2")).await.is_empty());
    assert_eq!(take(None).await, vec!["no session".to_string()]);
}

#[tokio::test]
async fn forget_drops_the_queue() {
    publish(Some("s3".into()), "gone".into()).await;
    forget("s3").await;
    assert!(take(Some("s3")).await.is_empty());
}
