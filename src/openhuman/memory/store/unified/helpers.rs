use crate::openhuman::memory::chunker::chunk_markdown;

use super::UnifiedMemory;

impl UnifiedMemory {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn write_markdown_doc(
        &self,
        namespace: &str,
        doc_id: &str,
        title: &str,
        source_type: &str,
        priority: &str,
        tags: &[String],
        created_at: f64,
        updated_at: f64,
        content: &str,
    ) -> anyhow::Result<String> {
        let docs_dir = self.namespace_dir(namespace).join("docs");
        std::fs::create_dir_all(&docs_dir)?;
        let rel_path = format!(
            "memory/namespaces/{}/docs/{doc_id}.md",
            Self::sanitize_namespace(namespace)
        );
        let abs_path = self.workspace_dir.join(&rel_path);

        let header = format!(
            "---\ndoc_id: {doc_id}\nnamespace: {}\ntitle: {}\nsource_type: {}\npriority: {}\ntags: {}\ncreated_at: {}\nupdated_at: {}\n---\n\n",
            namespace.replace('\n', " "),
            title.replace('\n', " "),
            source_type.replace('\n', " "),
            priority.replace('\n', " "),
            serde_json::to_string(tags).unwrap_or_else(|_| "[]".to_string()),
            created_at,
            updated_at
        );
        std::fs::write(abs_path, format!("{header}{content}\n"))?;
        Ok(rel_path)
    }

    pub(crate) fn vec_to_bytes(v: &[f32]) -> Vec<u8> {
        let mut bytes = Vec::with_capacity(v.len() * 4);
        for &f in v {
            bytes.extend_from_slice(&f.to_le_bytes());
        }
        bytes
    }

    pub(crate) fn bytes_to_vec(bytes: &[u8]) -> Vec<f32> {
        bytes
            .chunks_exact(4)
            .map(|chunk| {
                let arr: [u8; 4] = chunk.try_into().unwrap_or([0; 4]);
                f32::from_le_bytes(arr)
            })
            .collect()
    }

    pub(crate) fn cosine_similarity(a: &[f32], b: &[f32]) -> f64 {
        if a.len() != b.len() || a.is_empty() {
            return 0.0;
        }
        let mut dot = 0.0_f64;
        let mut norm_a = 0.0_f64;
        let mut norm_b = 0.0_f64;
        for (x, y) in a.iter().zip(b.iter()) {
            let x = f64::from(*x);
            let y = f64::from(*y);
            dot += x * y;
            norm_a += x * x;
            norm_b += y * y;
        }
        let denom = norm_a.sqrt() * norm_b.sqrt();
        if denom <= f64::EPSILON {
            return 0.0;
        }
        (dot / denom).clamp(0.0, 1.0)
    }

    pub(crate) fn chunk_document_content(content: &str, max_tokens: usize) -> Vec<String> {
        let mut chunks: Vec<String> = chunk_markdown(content, max_tokens.max(1))
            .into_iter()
            .map(|chunk| chunk.content.trim().to_string())
            .filter(|chunk: &String| !chunk.is_empty())
            .collect();
        if chunks.is_empty() && !content.trim().is_empty() {
            chunks.push(content.trim().to_string());
        }
        chunks
    }

    pub(crate) fn collapse_whitespace(text: &str) -> String {
        text.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    pub(crate) fn normalize_search_text(text: &str) -> String {
        let collapsed = Self::collapse_whitespace(text);
        let mut normalized = String::with_capacity(collapsed.len());
        for ch in collapsed.chars() {
            if ch.is_alphanumeric() {
                normalized.extend(ch.to_lowercase());
            } else if ch.is_whitespace() || matches!(ch, '_' | '-' | '/' | '.') {
                normalized.push(' ');
            }
        }
        normalized.split_whitespace().collect::<Vec<_>>().join(" ")
    }

    pub(crate) fn tokenize_search_terms(text: &str) -> Vec<String> {
        Self::normalize_search_text(text)
            .split_whitespace()
            .map(ToOwned::to_owned)
            .collect()
    }

    pub(crate) fn normalize_graph_entity(text: &str) -> String {
        Self::collapse_whitespace(text.trim()).to_uppercase()
    }

    pub(crate) fn normalize_graph_predicate(text: &str) -> String {
        let mut out = String::new();
        let mut last_was_sep = false;
        for ch in Self::collapse_whitespace(text.trim()).chars() {
            if ch.is_alphanumeric() {
                out.extend(ch.to_uppercase());
                last_was_sep = false;
            } else if !last_was_sep {
                out.push('_');
                last_was_sep = true;
            }
        }
        out.trim_matches('_').to_string()
    }

    pub(crate) fn json_string_array(
        value: &serde_json::Value,
        primary_key: &str,
        singular_key: &str,
    ) -> Vec<String> {
        let mut items = Vec::new();
        if let Some(array) = value.get(primary_key).and_then(serde_json::Value::as_array) {
            for item in array {
                if let Some(text) = item.as_str() {
                    let trimmed = text.trim();
                    if !trimmed.is_empty() {
                        items.push(trimmed.to_string());
                    }
                }
            }
        }
        if let Some(text) = value.get(singular_key).and_then(serde_json::Value::as_str) {
            let trimmed = text.trim();
            if !trimmed.is_empty() {
                items.push(trimmed.to_string());
            }
        }
        items.sort();
        items.dedup();
        items
    }

    pub(crate) fn merge_unique_string_arrays(
        current: &serde_json::Value,
        incoming: &serde_json::Value,
        primary_key: &str,
        singular_key: &str,
    ) -> Vec<String> {
        let mut merged = Self::json_string_array(current, primary_key, singular_key);
        merged.extend(Self::json_string_array(incoming, primary_key, singular_key));
        merged.sort();
        merged.dedup();
        merged
    }

    pub(crate) fn json_i64(value: &serde_json::Value, key: &str) -> Option<i64> {
        value.get(key).and_then(|raw| {
            raw.as_i64().or_else(|| {
                raw.as_u64()
                    .and_then(|v| i64::try_from(v).ok())
                    .or_else(|| raw.as_f64().map(|v| v as i64))
            })
        })
    }

    pub(crate) fn recency_score(updated_at: f64, now: f64) -> f64 {
        let age_secs = (now - updated_at).max(0.0);
        let age_hours = age_secs / 3600.0;
        (1.0 / (1.0 + age_hours / 24.0)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::UnifiedMemory;
    use serde_json::json;

    // ── vec_to_bytes / bytes_to_vec ──────────────────────────────────

    #[test]
    fn vec_bytes_roundtrip() {
        let original = vec![1.0_f32, 2.5, -3.0, 0.0];
        let bytes = UnifiedMemory::vec_to_bytes(&original);
        assert_eq!(bytes.len(), 16); // 4 floats * 4 bytes
        let back = UnifiedMemory::bytes_to_vec(&bytes);
        assert_eq!(back, original);
    }

    #[test]
    fn vec_to_bytes_empty() {
        let bytes = UnifiedMemory::vec_to_bytes(&[]);
        assert!(bytes.is_empty());
        let back = UnifiedMemory::bytes_to_vec(&bytes);
        assert!(back.is_empty());
    }

    // ── cosine_similarity ────────────────────────────────────────────

    #[test]
    fn cosine_similarity_identical_vectors() {
        let v = vec![1.0_f32, 0.0, 0.0];
        let sim = UnifiedMemory::cosine_similarity(&v, &v);
        assert!((sim - 1.0).abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_orthogonal_vectors() {
        let a = vec![1.0_f32, 0.0];
        let b = vec![0.0_f32, 1.0];
        let sim = UnifiedMemory::cosine_similarity(&a, &b);
        assert!(sim.abs() < 1e-6);
    }

    #[test]
    fn cosine_similarity_different_lengths_returns_zero() {
        let a = vec![1.0_f32, 0.0];
        let b = vec![1.0_f32, 0.0, 0.0];
        assert_eq!(UnifiedMemory::cosine_similarity(&a, &b), 0.0);
    }

    #[test]
    fn cosine_similarity_empty_vectors_returns_zero() {
        assert_eq!(UnifiedMemory::cosine_similarity(&[], &[]), 0.0);
    }

    #[test]
    fn cosine_similarity_zero_vector_returns_zero() {
        let a = vec![0.0_f32, 0.0];
        let b = vec![1.0_f32, 0.0];
        assert_eq!(UnifiedMemory::cosine_similarity(&a, &b), 0.0);
    }

    // ── collapse_whitespace ──────────────────────────────────────────

    #[test]
    fn collapse_whitespace_normalizes() {
        assert_eq!(
            UnifiedMemory::collapse_whitespace("  hello   world  "),
            "hello world"
        );
    }

    #[test]
    fn collapse_whitespace_empty() {
        assert_eq!(UnifiedMemory::collapse_whitespace(""), "");
    }

    // ── normalize_search_text ────────────────────────────────────────

    #[test]
    fn normalize_search_text_lowercases_and_strips_special() {
        let result = UnifiedMemory::normalize_search_text("Hello, World! @#$ test");
        assert_eq!(result, "hello world test");
    }

    #[test]
    fn normalize_search_text_preserves_separators() {
        let result = UnifiedMemory::normalize_search_text("path/to_file-name.txt");
        assert_eq!(result, "path to file name txt");
    }

    // ── tokenize_search_terms ────────────────────────────────────────

    #[test]
    fn tokenize_search_terms_splits_correctly() {
        let terms = UnifiedMemory::tokenize_search_terms("Hello World");
        assert_eq!(terms, vec!["hello", "world"]);
    }

    #[test]
    fn tokenize_search_terms_empty() {
        assert!(UnifiedMemory::tokenize_search_terms("").is_empty());
        assert!(UnifiedMemory::tokenize_search_terms("  @#$  ").is_empty());
    }

    // ── normalize_graph_entity / predicate ───────────────────────────

    #[test]
    fn normalize_graph_entity_uppercases() {
        assert_eq!(
            UnifiedMemory::normalize_graph_entity("  rust language  "),
            "RUST LANGUAGE"
        );
    }

    #[test]
    fn normalize_graph_predicate_underscores_separators() {
        assert_eq!(
            UnifiedMemory::normalize_graph_predicate("is written in"),
            "IS_WRITTEN_IN"
        );
    }

    #[test]
    fn normalize_graph_predicate_strips_trailing_underscores() {
        assert_eq!(UnifiedMemory::normalize_graph_predicate("  has -- "), "HAS");
    }

    // ── json_string_array ────────────────────────────────────────────

    #[test]
    fn json_string_array_from_array_and_singular() {
        let val = json!({"tags": ["a", "b"], "tag": "c"});
        let result = UnifiedMemory::json_string_array(&val, "tags", "tag");
        assert_eq!(result, vec!["a", "b", "c"]);
    }

    #[test]
    fn json_string_array_deduplicates() {
        let val = json!({"tags": ["a", "a"], "tag": "a"});
        let result = UnifiedMemory::json_string_array(&val, "tags", "tag");
        assert_eq!(result, vec!["a"]);
    }

    #[test]
    fn json_string_array_empty_when_missing() {
        let val = json!({});
        let result = UnifiedMemory::json_string_array(&val, "tags", "tag");
        assert!(result.is_empty());
    }

    #[test]
    fn json_string_array_filters_empty_strings() {
        let val = json!({"tags": ["", "  ", "valid"]});
        let result = UnifiedMemory::json_string_array(&val, "tags", "tag");
        assert_eq!(result, vec!["valid"]);
    }

    // ── merge_unique_string_arrays ───────────────────────────────────

    #[test]
    fn merge_unique_string_arrays_combines_and_deduplicates() {
        let a = json!({"tags": ["x", "y"]});
        let b = json!({"tags": ["y", "z"]});
        let merged = UnifiedMemory::merge_unique_string_arrays(&a, &b, "tags", "tag");
        assert_eq!(merged, vec!["x", "y", "z"]);
    }

    // ── json_i64 ─────────────────────────────────────────────────────

    #[test]
    fn json_i64_from_integer() {
        assert_eq!(UnifiedMemory::json_i64(&json!({"n": 42}), "n"), Some(42));
    }

    #[test]
    fn json_i64_from_float() {
        assert_eq!(UnifiedMemory::json_i64(&json!({"n": 3.9}), "n"), Some(3));
    }

    #[test]
    fn json_i64_missing_key() {
        assert_eq!(UnifiedMemory::json_i64(&json!({}), "n"), None);
    }

    #[test]
    fn json_i64_from_string_returns_none() {
        assert_eq!(UnifiedMemory::json_i64(&json!({"n": "42"}), "n"), None);
    }

    // ── recency_score ────────────────────────────────────────────────

    #[test]
    fn recency_score_current_time_is_one() {
        let now = 1_700_000_000.0;
        let score = UnifiedMemory::recency_score(now, now);
        assert!((score - 1.0).abs() < 1e-6);
    }

    #[test]
    fn recency_score_old_document_is_lower() {
        let now = 1_700_000_000.0;
        let one_day_ago = now - 86400.0;
        let score = UnifiedMemory::recency_score(one_day_ago, now);
        assert!(score < 1.0);
        assert!(score > 0.0);
    }

    #[test]
    fn recency_score_future_clamped_to_one() {
        let now = 1_700_000_000.0;
        let future = now + 86400.0;
        let score = UnifiedMemory::recency_score(future, now);
        assert!((score - 1.0).abs() < 1e-6);
    }

    // ── chunk_document_content ───────────────────────────────────────

    #[test]
    fn chunk_document_content_returns_nonempty_for_content() {
        let chunks = UnifiedMemory::chunk_document_content("Hello world. This is a test.", 100);
        assert!(!chunks.is_empty());
    }

    #[test]
    fn chunk_document_content_empty_input_returns_empty() {
        let chunks = UnifiedMemory::chunk_document_content("", 100);
        assert!(chunks.is_empty());
    }

    #[test]
    fn chunk_document_content_whitespace_only_returns_empty() {
        let chunks = UnifiedMemory::chunk_document_content("   \n  \t  ", 100);
        assert!(chunks.is_empty());
    }
}
