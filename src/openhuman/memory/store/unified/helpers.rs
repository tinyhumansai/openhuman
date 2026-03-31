use crate::openhuman::memory::chunker::chunk_markdown;

use super::UnifiedMemory;

impl UnifiedMemory {
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
