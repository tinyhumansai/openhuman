/**
 * Voice and dictation commands.
 */
import { invoke } from '@tauri-apps/api/core';

import { callCoreRpc } from '../../services/coreRpcClient';
import { CommandResponse, isTauri } from './common';
import { ConfigSnapshot } from './config';

export interface VoiceSpeechResult {
  /** Final text — cleaned by LLM post-processing when available. */
  text: string;
  /** Raw whisper output before LLM cleanup. */
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
  whisper_binary: string | null;
  piper_binary: string | null;
  stt_model_path: string | null;
  tts_voice_path: string | null;
  /** Whether the whisper model is loaded in-process (low-latency mode). */
  whisper_in_process: boolean;
  /** Whether LLM post-processing is enabled for transcription cleanup. */
  llm_cleanup_enabled: boolean;
}

export interface VoiceServerStatus {
  state: 'stopped' | 'idle' | 'recording' | 'transcribing';
  hotkey: string;
  activation_mode: 'tap' | 'push';
  transcription_count: number;
  last_error: string | null;
}

export interface VoiceServerSettings {
  auto_start: boolean;
  hotkey: string;
  activation_mode: 'tap' | 'push';
  skip_cleanup: boolean;
  min_duration_secs: number;
  /** RMS energy threshold for silence detection. Recordings below this are
   *  treated as silence and skipped to prevent whisper hallucinations. */
  silence_threshold: number;
  /** Custom vocabulary words to bias whisper toward (names, technical terms). */
  custom_dictionary: string[];
}

export async function openhumanVoiceStatus(): Promise<VoiceStatus> {
  return await callCoreRpc<VoiceStatus>({ method: 'openhuman.voice_status', params: {} });
}

export async function openhumanVoiceServerStatus(): Promise<CommandResponse<VoiceServerStatus>> {
  return await callCoreRpc<CommandResponse<VoiceServerStatus>>({
    method: 'openhuman.voice_server_status',
    params: {},
  });
}

export async function openhumanVoiceServerStart(params?: {
  hotkey?: string;
  activation_mode?: 'tap' | 'push';
  skip_cleanup?: boolean;
}): Promise<CommandResponse<VoiceServerStatus>> {
  return await callCoreRpc<CommandResponse<VoiceServerStatus>>({
    method: 'openhuman.voice_server_start',
    params: params ?? {},
  });
}

export async function openhumanVoiceServerStop(): Promise<CommandResponse<VoiceServerStatus>> {
  return await callCoreRpc<CommandResponse<VoiceServerStatus>>({
    method: 'openhuman.voice_server_stop',
    params: {},
  });
}

export async function openhumanGetVoiceServerSettings(): Promise<
  CommandResponse<VoiceServerSettings>
> {
  return await callCoreRpc<CommandResponse<VoiceServerSettings>>({
    method: 'openhuman.config_get_voice_server_settings',
    params: {},
  });
}

export async function openhumanUpdateVoiceServerSettings(update: {
  auto_start?: boolean;
  hotkey?: string;
  activation_mode?: 'tap' | 'push';
  skip_cleanup?: boolean;
  min_duration_secs?: number;
  silence_threshold?: number;
  custom_dictionary?: string[];
}): Promise<CommandResponse<ConfigSnapshot>> {
  return await callCoreRpc<CommandResponse<ConfigSnapshot>>({
    method: 'openhuman.config_update_voice_server_settings',
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

/**
 * Register (or re-register) the global dictation toggle hotkey.
 */
export async function registerDictationHotkey(shortcut: string): Promise<void> {
  if (!isTauri()) {
    console.debug('[dictation] registerDictationHotkey: skipped — not running in Tauri');
    return;
  }
  const normalizedShortcut = shortcut
    .trim()
    .replace(/\bCommandOrControl\b/gi, 'CmdOrCtrl')
    .replace(/\bCommand\b/gi, 'Cmd')
    .replace(/\bControl\b/gi, 'Ctrl')
    .replace(/\bOption\b/gi, 'Alt');

  console.debug(
    '[dictation] registerDictationHotkey: shortcut=%s normalized=%s',
    shortcut,
    normalizedShortcut
  );
  try {
    await invoke<void>('register_dictation_hotkey', { shortcut: normalizedShortcut });
  } catch (err) {
    console.warn('[dictation] registerDictationHotkey normalized registration failed', err);
    throw err;
  }
  console.debug('[dictation] registerDictationHotkey: done');
}

/**
 * Unregister the global dictation hotkey if one is active.
 */
export async function unregisterDictationHotkey(): Promise<void> {
  if (!isTauri()) {
    console.debug('[dictation] unregisterDictationHotkey: skipped — not running in Tauri');
    return;
  }
  console.debug('[dictation] unregisterDictationHotkey: invoking');
  await invoke<void>('unregister_dictation_hotkey');
  console.debug('[dictation] unregisterDictationHotkey: done');
}
