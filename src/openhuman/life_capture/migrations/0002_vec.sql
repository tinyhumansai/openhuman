-- 1536-dim vectors for OpenAI text-embedding-3-small.
CREATE VIRTUAL TABLE IF NOT EXISTS item_vectors USING vec0(
    item_id TEXT PRIMARY KEY,
    embedding float[1536]
);
