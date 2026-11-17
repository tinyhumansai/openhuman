/**
 * Voice and dictation commands.
 */
import { callCoreRpc } from '../../services/coreRpcClient';

export interface VoiceSpeechResult {
  /** Final text — cleaned by LLM post-processing when available. */
  text: string;
  /** Raw STT engine output before LLM cleanup. */
  raw_text: string;
  model_id: string;
}

export interface VoiceTtsResult {
  output_path: string;
  voice_id: string;
}

export interface VoiceStatus {
  stt_available: boolean;
  tts_available: boolean;
  stt_model_id: string;
  tts_voice_id: string;
  piper_binary: string | null;
  tts_voice_path: string | null;
  /** Whether LLM post-processing is enabled for transcription cleanup. */
  llm_cleanup_enabled: boolean;
  /** Resolved STT routing string — 'cloud' for the backend proxy, or the
   *  third-party slug selected by `voice_server.stt_engine`. */
  stt_engine: string;
  /** Why `stt_available` is false, when it is. Null when STT is usable. */
  stt_error: string | null;
  /** Currently selected TTS provider ('cloud' or 'piper'). */
  tts_provider: string;
}

/** Hosted speech-to-text engines, mirroring `config::schema::SttEngine`. */
export type SttEngine = 'backend' | 'elevenlabs' | 'openai';

export async function openhumanVoiceStatus(): Promise<VoiceStatus> {
  return await callCoreRpc<VoiceStatus>({ method: 'openhuman.voice_status', params: {} });
}

export interface VoiceProvidersUpdate {
  /** STT provider string: 'cloud' or '<slug>:<model>'. */
  stt_provider?: string;
  /** TTS provider string: 'cloud', 'piper', or '<slug>:<voice>'. */
  tts_provider?: string;
  stt_model?: string;
  tts_voice?: string;
}

export interface VoiceProvidersSnapshot {
  stt_provider: string;
  tts_provider: string;
  stt_model_id: string;
  tts_voice_id: string;
}

/**
 * Persist the STT / TTS provider selection. Maps to the
 * `openhuman.voice_set_providers` RPC, which validates each value against
 * the supported provider list and rejects unknown ids server-side.
 */
export async function openhumanVoiceSetProviders(
  update: VoiceProvidersUpdate
): Promise<VoiceProvidersSnapshot> {
  return await callCoreRpc<VoiceProvidersSnapshot>({
    method: 'openhuman.voice_set_providers',
    params: update,
  });
}

export async function openhumanVoiceTranscribe(
  audioPath: string,
  context?: string,
  skipCleanup?: boolean
): Promise<VoiceSpeechResult> {
  return await callCoreRpc<VoiceSpeechResult>({
    method: 'openhuman.voice_transcribe',
    params: { audio_path: audioPath, context, skip_cleanup: skipCleanup },
  });
}

export async function openhumanVoiceTranscribeBytes(
  audioBytes: number[],
  extension?: string,
  context?: string,
  skipCleanup?: boolean
): Promise<VoiceSpeechResult> {
  return await callCoreRpc<VoiceSpeechResult>({
    method: 'openhuman.voice_transcribe_bytes',
    params: { audio_bytes: audioBytes, extension, context, skip_cleanup: skipCleanup },
  });
}

export async function openhumanVoiceTts(
  text: string,
  outputPath?: string
): Promise<VoiceTtsResult> {
  return await callCoreRpc<VoiceTtsResult>({
    method: 'openhuman.voice_tts',
    params: { text, output_path: outputPath },
  });
}
