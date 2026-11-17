use super::*;

#[tokio::test]
async fn mock_model_embeds_through_the_facade() {
    let model = MockEmbeddingModel::new(8);
    let vectors = model
        .embed(&["fn main() {}".to_string(), "struct S;".to_string()])
        .await
        .unwrap();
    assert_eq!(vectors.len(), 2);
    assert!(vectors.iter().all(|vector| vector.len() == 8));
    let query = model.embed_query("main").await.unwrap();
    assert_eq!(query.len(), 8);
}

#[test]
fn signature_format_is_pinned() {
    // Stored vectors are partitioned by this string; a change here re-homes
    // every host's index, so it is asserted literally.
    assert_eq!(
        format_embedding_signature("openai", "text-embedding-3-small", 1536),
        "provider=openai;model=text-embedding-3-small;dims=1536"
    );
}
