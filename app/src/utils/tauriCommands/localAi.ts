/**
 * Local AI / Ollama commands.
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { CommandResponse, isTauri, tauriErrorMessage } from './common';

export interface LocalAiStatus {
  state: string;
  model_id: string;
  chat_model_id: string;
  vision_model_id: string;
  embedding_model_id: string;
  stt_model_id: string;
  tts_voice_id: string;
  quantization: string;
  vision_state: string;
  vision_mode: string;
  embedding_state: string;
  stt_state: string;
  tts_state: string;
  provider: string;
  download_progress?: number | null;
  downloaded_bytes?: number | null;
  total_bytes?: number | null;
  download_speed_bps?: number | null;
  eta_seconds?: number | null;
  warning?: string | null;
  error_detail?: string | null;
  error_category?: string | null;
  model_path?: string | null;
  active_backend: string;
  backend_reason?: string | null;
  last_latency_ms?: number | null;
  prompt_toks_per_sec?: number | null;
  gen_toks_per_sec?: number | null;
}

export interface LocalAiAssetStatus {
  state: string;
  id: string;
  provider: string;
  path?: string | null;
  warning?: string | null;
}

export interface LocalAiAssetsStatus {
  chat: LocalAiAssetStatus;
  vision: LocalAiAssetStatus;
  embedding: LocalAiAssetStatus;
  stt: LocalAiAssetStatus;
  tts: LocalAiAssetStatus;
  quantization: string;
}

export interface LocalAiDownloadProgressItem {
  id: string;
  provider: string;
  state: string;
  progress?: number | null;
  downloaded_bytes?: number | null;
  total_bytes?: number | null;
  speed_bps?: number | null;
  eta_seconds?: number | null;
  warning?: string | null;
  path?: string | null;
}

export interface LocalAiDownloadsProgress {
  state: string;
  warning?: string | null;
  progress?: number | null;
  downloaded_bytes?: number | null;
  total_bytes?: number | null;
  speed_bps?: number | null;
  eta_seconds?: number | null;
  chat: LocalAiDownloadProgressItem;
  vision: LocalAiDownloadProgressItem;
  embedding: LocalAiDownloadProgressItem;
  stt: LocalAiDownloadProgressItem;
  tts: LocalAiDownloadProgressItem;
}

export interface LocalAiEmbeddingResult {
  model_id: string;
  dimensions: number;
  vectors: number[][];
}

export interface LocalAiSpeechResult {
  text: string;
  model_id: string;
}

export interface LocalAiTtsResult {
  output_path: string;
  voice_id: string;
}

export interface LocalAiChatMessage {
  role: 'user' | 'assistant' | 'system';
  content: string;
}

export interface LocalAiChatResult {
  result: string;
}

export interface ReactionDecision {
  should_react: boolean;
  emoji: string | null;
}

export interface SentimentResult {
  emotion: string;
  valence: string;
  confidence: number;
}

export interface GifDecision {
  should_send_gif: boolean;
  search_query: string | null;
}

export interface TenorMediaFormat {
  url: string;
  dims: [number, number];
  size: number;
  duration?: number;
}

export interface TenorGifResult {
  id: string;
  title: string;
  contentDescription: string;
  url: string;
  media: {
    gif?: TenorMediaFormat;
    tinygif?: TenorMediaFormat;
    mediumgif?: TenorMediaFormat;
    mp4?: TenorMediaFormat;
    tinymp4?: TenorMediaFormat;
  };
  created: number;
}

export interface TenorSearchResult {
  results: TenorGifResult[];
  next: string;
}

export interface DeviceProfileResult {
  total_ram_bytes: number;
  cpu_count: number;
  cpu_brand: string;
  os_name: string;
  os_version: string;
  has_gpu: boolean;
  gpu_description: string | null;
}

export interface ModelPresetResult {
  tier: string;
  label: string;
  description: string;
  chat_model_id: string;
  vision_model_id: string;
  embedding_model_id: string;
  quantization: string;
  vision_mode: string;
  supports_screen_summary: boolean;
  target_ram_gb: number;
  min_ram_gb: number;
  approx_download_gb: number;
}

export interface PresetsResponse {
  presets: ModelPresetResult[];
  recommended_tier: string;
  current_tier: string;
  selected_tier?: string | null;
  device: DeviceProfileResult;
  /** When true the device is below the RAM floor and cloud fallback is the recommended default. */
  recommend_disabled?: boolean;
  /** Current value of `config.local_ai.runtime_enabled`. When false, cloud fallback is in use. */
  local_ai_enabled?: boolean;
}

export interface ApplyPresetResult {
  applied_tier: string;
  chat_model_id?: string;
  vision_model_id?: string;
  embedding_model_id?: string;
  quantization?: string;
  vision_mode?: string;
  local_ai_enabled?: boolean;
}

export type RepairAction =
  | { action: 'install_ollama' }
  | { action: 'start_server'; binary_path: string | null }
  | { action: 'pull_model'; model: string };

export interface LocalAiDiagnostics {
  ollama_running: boolean;
  ollama_base_url: string;
  ollama_binary_path: string | null;
  vision_mode?: string;
  installed_models: Array<{ name: string; size?: number | null; modified_at?: string | null }>;
  expected: {
    chat_model: string;
    chat_found: boolean;
    embedding_model: string;
    embedding_found: boolean;
    vision_model: string;
    vision_found: boolean;
  };
  issues: string[];
  repair_actions: RepairAction[];
  ok: boolean;
}

export async function openhumanAgentChat(
  message: string,
  modelOverride?: string,
  temperature?: number
): Promise<CommandResponse<string>> {
  if (!isTauri()) {
    throw new Error('Not running in Tauri');
  }
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.agent_chat',
    params: { message, model_override: modelOverride, temperature },
  });
}

export async function openhumanLocalAiStatus(): Promise<CommandResponse<LocalAiStatus>> {
  try {
    return await callCoreRpc<CommandResponse<LocalAiStatus>>({
      method: 'openhuman.local_ai_status',
    });
  } catch (err) {
    const message = tauriErrorMessage(err);
    if (message.includes('unknown method: openhuman.local_ai_status')) {
      throw new Error(
        'Local model runtime is unavailable in this core build. Restart app after updating to the latest build.'
      );
    }
    throw new Error(message);
  }
}

export async function openhumanLocalAiDownload(
  force?: boolean
): Promise<CommandResponse<LocalAiStatus>> {
  try {
    return await callCoreRpc<CommandResponse<LocalAiStatus>>({
      method: 'openhuman.local_ai_download',
      params: { force: force ?? false },
    });
  } catch (err) {
    const message = tauriErrorMessage(err);
    if (message.includes('unknown method: openhuman.local_ai_download')) {
      return await openhumanLocalAiStatus();
    }
    throw new Error(message);
  }
}

export async function openhumanLocalAiDownloadAllAssets(
  force?: boolean
): Promise<CommandResponse<LocalAiDownloadsProgress>> {
  return await callCoreRpc<CommandResponse<LocalAiDownloadsProgress>>({
    method: 'openhuman.local_ai_download_all_assets',
    params: { force: force ?? false },
  });
}

export async function openhumanLocalAiSummarize(
  text: string,
  maxTokens?: number
): Promise<CommandResponse<string>> {
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.local_ai_summarize',
    params: { text, max_tokens: maxTokens },
  });
}

export async function openhumanLocalAiPrompt(
  prompt: string,
  maxTokens?: number,
  noThink?: boolean
): Promise<CommandResponse<string>> {
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.local_ai_prompt',
    params: { prompt, max_tokens: maxTokens, no_think: noThink },
  });
}

export async function openhumanLocalAiVisionPrompt(
  prompt: string,
  imageRefs: string[],
  maxTokens?: number
): Promise<CommandResponse<string>> {
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.local_ai_vision_prompt',
    params: { prompt, image_refs: imageRefs, max_tokens: maxTokens },
  });
}

export async function openhumanLocalAiEmbed(
  inputs: string[]
): Promise<CommandResponse<LocalAiEmbeddingResult>> {
  return await callCoreRpc<CommandResponse<LocalAiEmbeddingResult>>({
    method: 'openhuman.local_ai_embed',
    params: { inputs },
  });
}

export async function openhumanLocalAiTranscribe(
  audioPath: string
): Promise<CommandResponse<LocalAiSpeechResult>> {
  return await callCoreRpc<CommandResponse<LocalAiSpeechResult>>({
    method: 'openhuman.local_ai_transcribe',
    params: { audio_path: audioPath },
  });
}

export async function openhumanLocalAiTranscribeBytes(
  audioBytes: number[],
  extension?: string
): Promise<CommandResponse<LocalAiSpeechResult>> {
  return await callCoreRpc<CommandResponse<LocalAiSpeechResult>>({
    method: 'openhuman.local_ai_transcribe_bytes',
    params: { audio_bytes: audioBytes, extension },
  });
}

export async function openhumanLocalAiTts(
  text: string,
  outputPath?: string
): Promise<CommandResponse<LocalAiTtsResult>> {
  return await callCoreRpc<CommandResponse<LocalAiTtsResult>>({
    method: 'openhuman.local_ai_tts',
    params: { text, output_path: outputPath },
  });
}

/**
 * Multi-turn chat completion via the local Ollama model.
 */
export async function openhumanLocalAiChat(
  messages: LocalAiChatMessage[],
  maxTokens?: number
): Promise<CommandResponse<string>> {
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.local_ai_chat',
    params: { messages, max_tokens: maxTokens },
  });
}

/**
 * Ask the local model whether the assistant should react to a user message
 * with an emoji.
 */
export async function openhumanLocalAiShouldReact(
  message: string,
  channelType: string
): Promise<CommandResponse<ReactionDecision>> {
  return await callCoreRpc<CommandResponse<ReactionDecision>>({
    method: 'openhuman.local_ai_should_react',
    params: { message, channel_type: channelType },
  });
}

/**
 * Classify the emotion and sentiment of a user message via the local model.
 */
export async function openhumanLocalAiAnalyzeSentiment(
  message: string
): Promise<CommandResponse<SentimentResult>> {
  return await callCoreRpc<CommandResponse<SentimentResult>>({
    method: 'openhuman.local_ai_analyze_sentiment',
    params: { message },
  });
}

/**
 * Ask the local model whether a GIF response is appropriate for this message.
 */
export async function openhumanLocalAiShouldSendGif(
  message: string,
  channelType: string
): Promise<CommandResponse<GifDecision>> {
  return await callCoreRpc<CommandResponse<GifDecision>>({
    method: 'openhuman.local_ai_should_send_gif',
    params: { message, channel_type: channelType },
  });
}

/**
 * Search for GIFs via the backend Tenor proxy.
 */
export async function openhumanLocalAiTenorSearch(
  query: string,
  limit?: number
): Promise<CommandResponse<TenorSearchResult>> {
  return await callCoreRpc<CommandResponse<TenorSearchResult>>({
    method: 'openhuman.local_ai_tenor_search',
    params: { query, limit },
  });
}

export async function openhumanLocalAiAssetsStatus(): Promise<
  CommandResponse<LocalAiAssetsStatus>
> {
  return await callCoreRpc<CommandResponse<LocalAiAssetsStatus>>({
    method: 'openhuman.local_ai_assets_status',
  });
}

export async function openhumanLocalAiDownloadsProgress(): Promise<
  CommandResponse<LocalAiDownloadsProgress>
> {
  return await callCoreRpc<CommandResponse<LocalAiDownloadsProgress>>({
    method: 'openhuman.local_ai_downloads_progress',
  });
}

export async function openhumanLocalAiDownloadAsset(
  capability: 'chat' | 'vision' | 'embedding' | 'stt' | 'tts'
): Promise<CommandResponse<LocalAiAssetsStatus>> {
  return await callCoreRpc<CommandResponse<LocalAiAssetsStatus>>({
    method: 'openhuman.local_ai_download_asset',
    params: { capability },
  });
}

export async function openhumanLocalAiDeviceProfile(): Promise<DeviceProfileResult> {
  return await callCoreRpc<DeviceProfileResult>({ method: 'openhuman.local_ai_device_profile' });
}

export async function openhumanLocalAiPresets(): Promise<PresetsResponse> {
  return await callCoreRpc<PresetsResponse>({ method: 'openhuman.local_ai_presets' });
}

export async function openhumanLocalAiApplyPreset(tier: string): Promise<ApplyPresetResult> {
  return await callCoreRpc<ApplyPresetResult>({
    method: 'openhuman.local_ai_apply_preset',
    params: { tier },
  });
}

export async function openhumanLocalAiDiagnostics(): Promise<LocalAiDiagnostics> {
  return await callCoreRpc<LocalAiDiagnostics>({
    method: 'openhuman.local_ai_diagnostics',
    params: {},
  });
}

export async function openhumanLocalAiSetOllamaPath(
  path: string
): Promise<{ ollama_binary_path: string | null; status: LocalAiStatus }> {
  return await callCoreRpc<{ ollama_binary_path: string | null; status: LocalAiStatus }>({
    method: 'openhuman.local_ai_set_ollama_path',
    params: { path },
  });
}
