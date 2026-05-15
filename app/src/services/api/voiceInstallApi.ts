/**
 * Voice engine installer API — wraps the four new `local_ai.*` RPCs that
 * orchestrate downloads of the Whisper GGML model + binary and the Piper
 * binary + bundled voice into the workspace.
 *
 * The renderer never touches HTTP URLs directly; everything funnels
 * through the Rust core where streaming + atomic rename + SHA validation
 * lives. From the UI's point of view a button click translates to a
 * single RPC kick-off plus a polled status RPC for progress.
 */
import debug from 'debug';

import { callCoreRpc } from '../coreRpcClient';

const log = debug('voiceInstallApi');

/**
 * Stable wire shape of [`crate::openhuman::local_ai::voice_install_common::VoiceInstallState`].
 *
 * The Rust enum serializes via `#[serde(rename_all = "snake_case")]` so
 * the TypeScript union mirrors the lowercase variants exactly.
 */
export type VoiceInstallState = 'missing' | 'installing' | 'installed' | 'broken' | 'error';

/**
 * Mirrors `VoiceInstallStatus` on the Rust side. The shape is identical
 * for both `whisper` and `piper` so the renderer can share components.
 */
export interface VoiceInstallStatus {
  /** `"whisper"` or `"piper"`. */
  engine: string;
  /** Current state — drives the button label / spinner. */
  state: VoiceInstallState;
  /** 0–100 percent, populated while `state === 'installing'`. */
  progress: number | null;
  /** Bytes received so far across the current download stage. */
  downloaded_bytes: number | null;
  /** Total bytes expected (may be null for chunked transfer encoding). */
  total_bytes: number | null;
  /** Free-text status line — e.g. "downloading model (ggml-tiny.bin)". */
  stage: string | null;
  /** Populated when `state === 'error'`. */
  error_detail: string | null;
}

export interface InstallWhisperParams {
  /** Whisper model size — `tiny | base | small | medium | large-v3-turbo`. */
  modelSize?: string;
  /** When true, blow away the existing model and re-download. */
  force?: boolean;
}

export interface InstallPiperParams {
  /** Piper voice id (e.g. `en_US-lessac-medium`). */
  voiceId?: string;
  /** When true, blow away the existing voice files and re-download. */
  force?: boolean;
}

/**
 * Kick off (or re-kick) a Whisper install. Resolves with the post-install
 * status snapshot — the renderer should also poll `whisperInstallStatus`
 * during the in-flight phase to update progress.
 */
export async function installWhisper(
  params: InstallWhisperParams = {}
): Promise<VoiceInstallStatus> {
  log('[voice-install:whisper] kick-off %o', params);
  const result = await callCoreRpc<VoiceInstallStatus>({
    method: 'openhuman.local_ai_install_whisper',
    params: { model_size: params.modelSize, force: params.force },
  });
  log('[voice-install:whisper] result state=%s stage=%s', result.state, result.stage ?? '<none>');
  return result;
}

/**
 * Kick off (or re-kick) a Piper install. See `installWhisper` for the
 * mental model — same wire shape, different engine slot.
 */
export async function installPiper(params: InstallPiperParams = {}): Promise<VoiceInstallStatus> {
  log('[voice-install:piper] kick-off %o', params);
  const result = await callCoreRpc<VoiceInstallStatus>({
    method: 'openhuman.local_ai_install_piper',
    params: { voice_id: params.voiceId, force: params.force },
  });
  log('[voice-install:piper] result state=%s stage=%s', result.state, result.stage ?? '<none>');
  return result;
}

/**
 * Query the current Whisper installer state. Safe to call repeatedly —
 * the core returns from an in-memory status table without touching disk
 * unless the table is empty (first read after a process restart), in
 * which case it falls back to a one-shot on-disk artifact check.
 */
export async function whisperInstallStatus(): Promise<VoiceInstallStatus> {
  return await callCoreRpc<VoiceInstallStatus>({
    method: 'openhuman.local_ai_whisper_install_status',
    params: {},
  });
}

/**
 * Query the current Piper installer state. Same contract as
 * `whisperInstallStatus`.
 */
export async function piperInstallStatus(): Promise<VoiceInstallStatus> {
  return await callCoreRpc<VoiceInstallStatus>({
    method: 'openhuman.local_ai_piper_install_status',
    params: {},
  });
}
