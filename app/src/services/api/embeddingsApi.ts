/**
 * Embeddings settings API — facade for the Settings → Embeddings panel.
 *
 * Wraps the `openhuman.embeddings_*` RPC methods. The panel never imports
 * `coreRpcClient` directly — every call goes through this file.
 */
import { callCoreRpc } from '../coreRpcClient';
import { CORE_RPC_METHODS } from '../rpcMethods';

// ─── Domain types ────────────────────────────────────────────────────────────

export interface EmbeddingModelPreset {
  id: string;
  label: string;
  default_dimensions: number;
  allowed_dimensions: number[];
}

export interface EmbeddingProviderEntry {
  slug: string;
  label: string;
  description: string;
  requires_api_key: boolean;
  requires_endpoint: boolean;
  has_api_key: boolean;
  models: EmbeddingModelPreset[];
}

export interface EmbeddingsSettings {
  /** The picker's own setting (`config.memory.embedding_provider`). */
  provider: string;
  /**
   * The embedder ingestion will **actually** use, resolved core-side by the
   * memory-tree provider ladder (#5402). Ask this — not `provider` — when the
   * question is "do these embeddings bill against the managed budget?": the
   * Local AI "Memory embeddings" toggle and the `memory_tree.embedding_endpoint`
   * override both route to local Ollama without rewriting `provider`.
   *
   * One of `ollama` | `custom` | `cloud` | `none` | `unconfigured` | `unknown`.
   * Optional so an older core (or a stubbed test payload) degrades to `provider`
   * rather than throwing.
   */
  effective_provider?: string;
  model: string;
  dimensions: number;
  rate_limit_per_min: number;
  providers: EmbeddingProviderEntry[];
  vector_search_enabled: boolean;
}

interface EmbeddingsUpdateResult {
  provider?: string;
  model?: string;
  dimensions?: number;
  signature_changed?: boolean;
  new_signature?: string;
  /** Present when confirm_wipe was required but not supplied */
  error?: string;
  message?: string;
  /** Underlying probe failure (HTTP status / server error body) for diagnosis (#4056) */
  detail?: string;
  old_signature?: string;
}

export interface EmbeddingsTestResult {
  success: boolean;
  provider: string;
  model: string;
  requested_dimensions?: number;
  actual_dimensions?: number;
  error?: string;
}

// ─── API calls ───────────────────────────────────────────────────────────────

export async function loadEmbeddingsSettings(): Promise<EmbeddingsSettings> {
  const raw = await callCoreRpc<EmbeddingsSettings | { result: EmbeddingsSettings }>({
    method: CORE_RPC_METHODS.embeddingsGetSettings,
    params: {},
  });
  return 'result' in raw ? raw.result : raw;
}

export async function updateEmbeddingsSettings(params: {
  provider?: string;
  model?: string;
  dimensions?: number;
  custom_endpoint?: string;
  rate_limit_per_min?: number;
  confirm_wipe?: boolean;
}): Promise<EmbeddingsUpdateResult> {
  const raw = await callCoreRpc<EmbeddingsUpdateResult | { result: EmbeddingsUpdateResult }>({
    method: CORE_RPC_METHODS.embeddingsUpdateSettings,
    params,
  });
  return 'result' in raw ? raw.result : raw;
}

export async function setEmbeddingsApiKey(provider: string, apiKey: string): Promise<void> {
  await callCoreRpc({
    method: CORE_RPC_METHODS.embeddingsSetApiKey,
    params: { provider, api_key: apiKey },
  });
}

export async function clearEmbeddingsApiKey(provider: string): Promise<void> {
  await callCoreRpc({ method: CORE_RPC_METHODS.embeddingsClearApiKey, params: { provider } });
}

export async function testEmbeddingsConnection(params?: {
  provider?: string;
  model?: string;
  dimensions?: number;
}): Promise<EmbeddingsTestResult> {
  const raw = await callCoreRpc<EmbeddingsTestResult | { result: EmbeddingsTestResult }>({
    method: CORE_RPC_METHODS.embeddingsTestConnection,
    params: params ?? {},
  });
  return 'result' in raw ? raw.result : raw;
}
