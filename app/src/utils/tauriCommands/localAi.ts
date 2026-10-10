/**
 * Local AI / Ollama-facing commands routed through the core.
 *
 * The renderer never talks to Ollama directly. It always calls the core, and
 * the core decides whether to route a request to the configured inference
 * backend (for example an external Ollama endpoint).
 */
import { callCoreRpc } from '../../services/coreRpcClient';
import { CommandResponse, tauriErrorMessage } from './common';

export interface LocalAiStatus {
  /**
   * Runtime state of the user-run local endpoint: `ready`, `degraded`,
   * `unreachable`, `disabled` or `idle`. The app never downloads or installs
   * models or runtimes, so there is no download/install state.
   */
  state: string;
  model_id: string;
  chat_model_id: string;
  vision_model_id: string;
  embedding_model_id: string;
  stt_model_id: string;
  tts_voice_id: string;
  vision_state: string;
  vision_mode: string;
  embedding_state: string;
  stt_state: string;
  tts_state: string;
  provider: string;
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

export interface LocalAiSpeechResult {
  text: string;
  model_id: string;
}

export interface LocalAiTtsResult {
  output_path: string;
  voice_id: string;
}

export interface SentimentResult {
  emotion: string;
  valence: string;
  confidence: number;
}

/**
 * Always empty: the core no longer installs or starts runtimes or pulls
 * models, so remediation is text-only in `issues`.
 */
export type RepairAction = never;

/**
 * Verdict for a model's native context window against the memory-layer
 * minimum. Mirrors the Rust `ContextEligibility` enum (serde tagged by
 * `status`). `below_minimum` means the model is rejected for memory-layer
 * use; `unknown` means the context window could not be determined (not a
 * hard rejection).
 */
export type ModelContextEligibility =
  | { status: 'ok'; context_length: number }
  | { status: 'below_minimum'; context_length: number; required: number }
  | { status: 'unknown'; required: number };

export interface InstalledModelInfo {
  name: string;
  size?: number | null;
  modified_at?: string | null;
  /** Native context window in tokens, or null when `/api/show` didn't report it. */
  context_length?: number | null;
  eligibility?: ModelContextEligibility | null;
  /**
   * Whether the model can serve chat/completions (from Ollama `/api/show`
   * `capabilities`). `false` = embedding-only model that must be hidden from
   * the chat-model picker; `null` = unknown (older Ollama / `/api/show` miss),
   * treated as visible (fail-open). See Sentry TAURI-RUST-4P6.
   */
  chat_capable?: boolean | null;
}

export interface LocalAiDiagnostics {
  ollama_running: boolean;
  /** Fine-grained status from the two-phase health probe (#6032). */
  ollama_status?: 'running' | 'degraded' | 'stopped';
  ollama_runner_ok?: boolean;
  ollama_base_url: string;
  ollama_binary_path: string | null;
  vision_mode?: string;
  installed_models: InstalledModelInfo[];
  /** Memory-layer minimum a model's context window must meet to be accepted. */
  context_requirement?: { min_context_tokens: number };
  expected: {
    chat_model: string;
    chat_found: boolean;
    chat_eligibility?: ModelContextEligibility | null;
    embedding_model: string;
    embedding_found: boolean;
    embedding_eligibility?: ModelContextEligibility | null;
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
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.agent_chat',
    params: { message, model_override: modelOverride, temperature },
  });
}

export async function openhumanLocalAiStatus(): Promise<CommandResponse<LocalAiStatus>> {
  try {
    return await callCoreRpc<CommandResponse<LocalAiStatus>>({
      method: 'openhuman.inference_status',
    });
  } catch (err) {
    const message = tauriErrorMessage(err);
    if (message.includes('unknown method: openhuman.inference_status')) {
      throw new Error(
        'Local model runtime is unavailable in this core build. Restart app after updating to the latest build.'
      );
    }
    throw new Error(message);
  }
}

export async function openhumanLocalAiSummarize(
  text: string,
  maxTokens?: number
): Promise<CommandResponse<string>> {
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.inference_summarize',
    params: { text, max_tokens: maxTokens },
  });
}

export async function openhumanLocalAiPrompt(
  prompt: string,
  maxTokens?: number,
  noThink?: boolean
): Promise<CommandResponse<string>> {
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.inference_prompt',
    params: { prompt, max_tokens: maxTokens, no_think: noThink },
  });
}

export async function openhumanLocalAiVisionPrompt(
  prompt: string,
  imageRefs: string[],
  maxTokens?: number
): Promise<CommandResponse<string>> {
  return await callCoreRpc<CommandResponse<string>>({
    method: 'openhuman.inference_vision_prompt',
    params: { prompt, image_refs: imageRefs, max_tokens: maxTokens },
  });
}

export async function openhumanLocalAiTranscribe(
  audioPath: string
): Promise<CommandResponse<LocalAiSpeechResult>> {
  return await callCoreRpc<CommandResponse<LocalAiSpeechResult>>({
    method: 'openhuman.inference_transcribe',
    params: { audio_path: audioPath },
  });
}

export async function openhumanLocalAiTranscribeBytes(
  audioBytes: number[],
  extension?: string
): Promise<CommandResponse<LocalAiSpeechResult>> {
  return await callCoreRpc<CommandResponse<LocalAiSpeechResult>>({
    method: 'openhuman.inference_transcribe_bytes',
    params: { audio_bytes: audioBytes, extension },
  });
}

export async function openhumanLocalAiTts(
  text: string,
  outputPath?: string
): Promise<CommandResponse<LocalAiTtsResult>> {
  return await callCoreRpc<CommandResponse<LocalAiTtsResult>>({
    method: 'openhuman.inference_tts',
    params: { text, output_path: outputPath },
  });
}

/**
 * Classify the emotion and sentiment of a user message via the configured
 * inference provider.
 */
export async function openhumanLocalAiAnalyzeSentiment(
  message: string
): Promise<CommandResponse<SentimentResult>> {
  return await callCoreRpc<CommandResponse<SentimentResult>>({
    method: 'openhuman.inference_analyze_sentiment',
    params: { message },
  });
}

export async function openhumanLocalAiDiagnostics(): Promise<LocalAiDiagnostics> {
  return await callCoreRpc<LocalAiDiagnostics>({
    method: 'openhuman.inference_diagnostics',
    params: {},
  });
}

export interface OllamaConnectionTestResult {
  reachable: boolean;
  error?: string | null;
  models_count?: number | null;
}

export async function openhumanLocalAiTestConnection(
  url: string
): Promise<OllamaConnectionTestResult> {
  return await callCoreRpc<OllamaConnectionTestResult>({
    method: 'openhuman.inference_test_connection',
    params: { url },
  });
}
