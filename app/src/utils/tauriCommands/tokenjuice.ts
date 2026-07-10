/**
 * TokenJuice content-router client.
 *
 * Thin wrapper around the `openhuman.tokenjuice_*` JSON-RPC methods exposed by
 * the Rust core (`src/openhuman/tokenjuice/schemas.rs`): read/update the
 * `[tokenjuice]` settings block and read/reset compaction savings statistics.
 */
import { callCoreRpc } from '../../services/coreRpcClient';

/** The `[tokenjuice]` config block (snake_case, matching the Rust config keys). */
export interface TokenjuiceSettings {
  router_enabled: boolean;
  ccr_enabled: boolean;
  ccr_disk_enabled: boolean;
  max_cache_entries: number;
  max_cache_bytes: number;
  ccr_ttl_secs: number | null;
  min_bytes_to_compress: number;
  ccr_min_tokens: number;
  search_enabled: boolean;
  code_enabled: boolean;
  html_enabled: boolean;
  ml_compression_enabled: boolean;
  ml_model_id: string;
  ml_target_ratio: number;
  ml_sidecar_idle_timeout_secs: number;
  ml_max_input_chars: number;
  ml_device: string;
}

/** Partial update — only present fields are changed. */
export type TokenjuiceSettingsPatch = Partial<TokenjuiceSettings>;

export interface SavingsBucket {
  events: number;
  originalTokens: number;
  compactedTokens: number;
  tokensSaved: number;
  costSavedUsd: number;
}

export interface SavingsStats {
  attributionModel: string;
  total: SavingsBucket;
  byModel: Record<string, SavingsBucket>;
  byCompressor: Record<string, SavingsBucket>;
  cache: { entries: number; bytes: number };
}

export async function getTokenjuiceSettings(): Promise<TokenjuiceSettings> {
  const res = await callCoreRpc<{ settings: TokenjuiceSettings }>({
    method: 'openhuman.tokenjuice_settings_get',
    params: {},
  });
  return res.settings;
}

export async function updateTokenjuiceSettings(
  patch: TokenjuiceSettingsPatch
): Promise<TokenjuiceSettings> {
  const res = await callCoreRpc<{ settings: TokenjuiceSettings }>({
    method: 'openhuman.tokenjuice_settings_update',
    params: { patch },
  });
  return res.settings;
}

export async function getTokenjuiceSavings(): Promise<SavingsStats> {
  return await callCoreRpc<SavingsStats>({
    method: 'openhuman.tokenjuice_savings_stats',
    params: {},
  });
}

export async function resetTokenjuiceSavings(): Promise<void> {
  await callCoreRpc<{ ok: boolean }>({ method: 'openhuman.tokenjuice_savings_reset', params: {} });
}
