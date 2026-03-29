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

    pub(crate) fn split_chunks(content: &str, max_len: usize) -> Vec<String> {
        let mut out = Vec::new();
        let mut current = String::new();
        for para in content.split("\n\n") {
            let p = para.trim();
            if p.is_empty() {
                continue;
            }
            if current.is_empty() {
                current.push_str(p);
                continue;
            }
            if current.len() + 2 + p.len() <= max_len {
                current.push_str("\n\n");
                current.push_str(p);
            } else {
                out.push(std::mem::take(&mut current));
                current.push_str(p);
            }
        }
        if !current.trim().is_empty() {
            out.push(current);
        }
        out
    }
}
