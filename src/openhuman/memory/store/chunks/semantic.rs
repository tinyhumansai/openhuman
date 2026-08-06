//! Compatibility exports for tinycortex's semantic Markdown chunker.

pub use tinycortex::memory::chunks::SemanticChunk as Chunk;

pub fn chunk_markdown(text: &str, max_tokens: usize) -> Vec<Chunk> {
    tinycortex::memory::chunks::chunk_semantic(text, max_tokens)
}
