/**
 * Settings tab API layer for the Intelligence page.
 *
 * Wraps the existing `local_ai_*` core RPCs (re-exported with cleaner names)
 * and the canonical `openhuman.memory_tree_get_llm` / `set_llm` JSON-RPC
 * methods that drive the AI-backend selector. Both come from the shared
 * `utils/tauriCommands` barrel.
 *
 * Logging convention: `[intelligence-settings-api]` prefix for grep-friendly
 * tracing of the new flow per the project debug-logging rule.
 */
import {
  type LlmBackend,
  type LocalAiAssetsStatus,
  type LocalAiDiagnostics,
  type LocalAiStatus,
  memoryTreeGetLlm,
  memoryTreeSetLlm,
  openhumanLocalAiAssetsStatus,
  openhumanLocalAiDiagnostics,
  openhumanLocalAiDownloadAsset,
  openhumanLocalAiPresets,
  openhumanLocalAiStatus,
  type PresetsResponse,
} from '../../utils/tauriCommands';

/**
 * AI backend the assistant is currently using for chat. Re-exports the
 * canonical `LlmBackend` from the wrapper so both names remain valid as
 * call-sites migrate.
 */
export type Backend = LlmBackend;

/** Static descriptor used by ModelAssignment + ModelCatalog. */
export interface ModelDescriptor {
  /** Ollama-style identifier (e.g. `qwen2.5:0.5b`). */
  id: string;
  /** Pretty label shown in the UI (defaults to `id` when omitted). */
  label?: string;
  /** Human-readable disk size, e.g. `400 MB`. */
  size: string;
  /** Bytes — approximate; surfaced for sort / filter. */
  approxBytes: number;
  /** Approx RAM hint, e.g. `≤4 GB RAM`. */
  ramHint: string;
  /** Speed / quality tier — used for the inline annotation under each row. */
  category: 'fast' | 'balanced' | 'high quality' | 'embedder';
  /** One-sentence note about when to pick this model. */
  note: string;
  /** Role(s) this model is suitable for. */
  roles: ReadonlyArray<'extract' | 'summariser' | 'embedder'>;
}

export type ModelRole = 'extract' | 'summariser' | 'embedder';

/**
 * Hard-coded recommended catalog. In a future wave this should come from
 * a `local_ai.recommended_catalog` RPC; for v1 we ship a curated list so
 * the UI is fully populated without a server roundtrip.
 */
export const RECOMMENDED_MODEL_CATALOG: ReadonlyArray<ModelDescriptor> = [
  {
    id: 'qwen2.5:0.5b',
    size: '400 MB',
    approxBytes: 400 * 1024 * 1024,
    ramHint: '≤4 GB RAM',
    category: 'fast',
    note: 'compact, lower quality',
    roles: ['extract'],
  },
  {
    id: 'gemma3:1b-it-qat',
    size: '1.0 GB',
    approxBytes: Math.round(1.0 * 1024 * 1024 * 1024),
    ramHint: '≤4 GB RAM',
    category: 'fast',
    note: 'compact Gemma; OK on laptops without a GPU',
    roles: ['extract', 'summariser'],
  },
  {
    id: 'gemma3:4b',
    size: '3.3 GB',
    approxBytes: Math.round(3.3 * 1024 * 1024 * 1024),
    ramHint: '≤8 GB RAM',
    category: 'balanced',
    note: 'default summariser — coherent abstractive output',
    roles: ['extract', 'summariser'],
  },
  {
    id: 'gemma3:12b-it-qat',
    size: '8.9 GB',
    approxBytes: Math.round(8.9 * 1024 * 1024 * 1024),
    ramHint: '≥16 GB RAM',
    category: 'high quality',
    note: 'larger Gemma; sharper summaries on capable hardware',
    roles: ['extract', 'summariser'],
  },
  {
    id: 'bge-m3',
    size: '1.3 GB',
    approxBytes: Math.round(1.3 * 1024 * 1024 * 1024),
    ramHint: '≥4 GB RAM',
    category: 'embedder',
    note: 'required for embeddings',
    roles: ['embedder'],
  },
];

export const DEFAULT_EXTRACT_MODEL = 'gemma3:4b';
export const DEFAULT_SUMMARISER_MODEL = 'gemma3:4b';
export const REQUIRED_EMBEDDER_MODEL = 'bge-m3';

/**
 * Reads the currently configured chat backend from the core.
 *
 * Backed by `openhuman.memory_tree_get_llm` — the value persists across
 * sidecar restarts via `config.toml`.
 */
export async function getMemoryTreeLlm(): Promise<Backend> {
  console.debug('[intelligence-settings-api] getMemoryTreeLlm: entry');
  const resp = await memoryTreeGetLlm();
  console.debug('[intelligence-settings-api] getMemoryTreeLlm: exit current=%s', resp.current);
  return resp.current;
}

/**
 * Optional per-role model picks for {@link setMemoryTreeLlm}. Field names
 * are camelCase here to match TS conventions; the wrapper translates them
 * to the snake_case wire shape the Rust `SetLlmRequest` expects:
 *
 * | TS option         | Rust / wire field   | Targets `memory_tree.*` |
 * | ----------------- | ------------------- | ----------------------- |
 * | `cloudModel`      | `cloud_model`       | `cloud_llm_model`       |
 * | `extractModel`    | `extract_model`     | `llm_extractor_model`   |
 * | `summariserModel` | `summariser_model`  | `llm_summariser_model`  |
 *
 * Each field follows "absent → unchanged, present → overwritten" so a
 * caller flipping just the backend doesn't have to re-supply every model
 * id, and a caller persisting just one role doesn't have to re-supply
 * the others.
 */
export interface SetMemoryTreeLlmOptions {
  cloudModel?: string;
  extractModel?: string;
  summariserModel?: string;
}

/**
 * Switches the chat backend and (optionally) persists per-role model
 * choices in the same atomic `config.toml` write. Returns the effective
 * value the core agreed on — today the handler accepts the input
 * verbatim, but a future revision may downgrade `local` → `cloud` when
 * the host can't satisfy the local minimums.
 *
 * Backed by `openhuman.memory_tree_set_llm`.
 *
 * Existing one-arg callers — `setMemoryTreeLlm('cloud')` — keep working
 * unchanged because `options` is optional.
 */
export async function setMemoryTreeLlm(
  next: Backend,
  options?: SetMemoryTreeLlmOptions
): Promise<{ effective: Backend }> {
  console.debug(
    '[intelligence-settings-api] setMemoryTreeLlm: entry next=%s cloudModel=%s extractModel=%s summariserModel=%s',
    next,
    options?.cloudModel ?? '<none>',
    options?.extractModel ?? '<none>',
    options?.summariserModel ?? '<none>'
  );
  // camelCase → snake_case translation lives here, in one place. The
  // wrapper layer just forwards the snake_case shape to the wire.
  const resp = await memoryTreeSetLlm({
    backend: next,
    ...(options?.cloudModel !== undefined && { cloud_model: options.cloudModel }),
    ...(options?.extractModel !== undefined && { extract_model: options.extractModel }),
    ...(options?.summariserModel !== undefined && { summariser_model: options.summariserModel }),
  });
  console.debug('[intelligence-settings-api] setMemoryTreeLlm: exit effective=%s', resp.current);
  return { effective: resp.current };
}

/** Re-export the existing assets status fetch with a friendlier name. */
export async function fetchInstalledAssets(): Promise<LocalAiAssetsStatus | null> {
  try {
    const response = await openhumanLocalAiAssetsStatus();
    return response.result;
  } catch (err) {
    console.debug('[intelligence-settings-api] fetchInstalledAssets failed', err);
    return null;
  }
}

/**
 * Fetch local AI status (includes per-capability state + last latency).
 * Used by `CurrentlyLoaded` to render Ollama-side telemetry.
 */
export async function fetchLocalAiStatus(): Promise<LocalAiStatus | null> {
  try {
    const response = await openhumanLocalAiStatus();
    return response.result;
  } catch (err) {
    console.debug('[intelligence-settings-api] fetchLocalAiStatus failed', err);
    return null;
  }
}

/**
 * Reach into the existing diagnostics RPC for the list of installed Ollama
 * models. The diagnostics endpoint already enumerates them and is the
 * cleanest single source of truth — we do not duplicate the model table.
 */
export async function fetchInstalledModels(): Promise<LocalAiDiagnostics['installed_models']> {
  try {
    const response = await openhumanLocalAiDiagnostics();
    return response.installed_models ?? [];
  } catch (err) {
    console.debug('[intelligence-settings-api] fetchInstalledModels failed', err);
    return [];
  }
}

export async function fetchPresets(): Promise<PresetsResponse | null> {
  try {
    return await openhumanLocalAiPresets();
  } catch (err) {
    console.debug('[intelligence-settings-api] fetchPresets failed', err);
    return null;
  }
}

/**
 * Trigger a download for a capability (chat / vision / embedding / stt / tts).
 * Used by ModelCatalog when the user clicks "Download".
 *
 * NOTE: the real RPC is per-capability, not per-model-id, so the catalog
 * picks the closest matching capability. This is acceptable for v1; future
 * iterations can swap in a per-model RPC.
 */
export async function downloadAsset(
  capability: 'chat' | 'vision' | 'embedding' | 'stt' | 'tts'
): Promise<LocalAiAssetsStatus | null> {
  try {
    const response = await openhumanLocalAiDownloadAsset(capability);
    return response.result;
  } catch (err) {
    console.debug('[intelligence-settings-api] downloadAsset failed', { capability, err });
    return null;
  }
}

/** Map a model descriptor to the closest capability bucket the core exposes. */
export function capabilityForModel(model: ModelDescriptor): 'chat' | 'embedding' | null {
  if (model.roles.includes('embedder')) return 'embedding';
  if (model.roles.includes('extract') || model.roles.includes('summariser')) return 'chat';
  return null;
}

/**
 * Cheap pretty-printer for a byte count. Mirrors the `JetBrains Mono`-style
 * compact format we want in the technical-readout sections.
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return '—';
  const gb = bytes / (1024 * 1024 * 1024);
  if (gb >= 1) return `${gb.toFixed(1)} GB`;
  const mb = bytes / (1024 * 1024);
  return `${Math.round(mb)} MB`;
}
