use super::*;

/// Deterministic stub provider: emits one fixed-length vector per input,
/// each component seeded from the input length so distinct texts map to
/// distinct vectors and the round-trip / dimension assertions are stable.
struct StubProvider {
    dims: usize,
}

#[async_trait]
impl EmbeddingModel for StubProvider {
    fn name(&self) -> &str {
        "stub"
    }

    fn model_id(&self) -> &str {
        "stub-model"
    }

    fn dimensions(&self) -> usize {
        self.dims
    }

    async fn embed(&self, texts: &[String]) -> EmbedResult<Vec<Vec<f32>>> {
        Ok(texts
            .iter()
            .map(|t| vec![t.len() as f32; self.dims])
            .collect())
    }
}

#[tokio::test]
async fn embeddings_adapter_round_trips_and_reports_dimensions() {
    let provider: Arc<dyn EmbeddingModel> = Arc::new(StubProvider { dims: 4 });
    let adapter = ProviderEmbeddingModel::new(provider, DispatchCtx::ambient());

    // dimensions() is forwarded from the underlying provider.
    assert_eq!(EmbeddingModel::name(&adapter), "stub");
    assert_eq!(EmbeddingModel::model_id(&adapter), "stub-model");
    assert_eq!(EmbeddingModel::dimensions(&adapter), 4);

    // embed() returns one vector per
    // input, in order, each of the reported dimensionality.
    let inputs = vec!["ab".to_string(), "abcd".to_string()];
    let vectors = adapter.embed(&inputs).await.expect("embed should succeed");
    assert_eq!(vectors.len(), 2);
    assert!(vectors.iter().all(|v| v.len() == 4));
    // Distinct inputs -> distinct vectors (seeded from text length).
    assert_eq!(vectors[0], vec![2.0; 4]);
    assert_eq!(vectors[1], vec![4.0; 4]);
}

#[tokio::test]
async fn embeddings_adapter_preserves_signature() {
    let provider: Arc<dyn EmbeddingModel> = Arc::new(StubProvider { dims: 8 });
    let expected = provider.signature();
    let adapter = ProviderEmbeddingModel::new(provider, DispatchCtx::ambient());
    assert_eq!(adapter.signature(), expected);
    assert_eq!(adapter.signature(), "provider=stub;model=stub-model;dims=8");
}

#[tokio::test]
async fn embeddings_adapter_empty_batch_is_empty() {
    let provider: Arc<dyn EmbeddingModel> = Arc::new(StubProvider { dims: 3 });
    let adapter = ProviderEmbeddingModel::new(provider, DispatchCtx::ambient());
    let vectors = adapter.embed(&[]).await.expect("empty embed ok");
    assert!(vectors.is_empty());
}
