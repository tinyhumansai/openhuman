# Memory tree embedding bridge

The memory tree stores fixed 1024-dimensional vectors. Concrete provider
transports live in TinyAgents; this directory keeps only memory-tree policy and
compatibility:

- `factory.rs`: read/write provider resolution, cloud-session policy, and
  degraded-state behavior;
- `openai_compat.rs`: OpenHuman config/credential and custom-slug resolution;
- `inert.rs`: deterministic 1024-element zero vectors for opt-out/tests;
- `mod.rs`: the legacy `Embedder` contract, `ProviderEmbedder` bridge, batch
  fallback/dimension checks, cosine math, and SQLite f32 packing helpers.

Ollama uses TinyAgents `OllamaEmbeddingModel` and `/api/embed`, with the shared
8192-token context and batch window. Managed cloud uses the host credential and
privacy wrapper around TinyAgents `CloudEmbeddingModel`. Do not add provider
HTTP clients in this directory.
