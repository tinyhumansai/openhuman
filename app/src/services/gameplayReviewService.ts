import debug from 'debug';

import { callCoreRpc } from './coreRpcClient';

const log = debug('gameplay-review');

export type SpoilerMode = 'off' | 'light' | 'full';

export interface GameplayFrameInput {
  file_name: string;
  image_ref: string;
  captured_at_ms?: number | null;
}

export interface GameplayReviewSessionInput {
  game_id: string;
  session_title: string;
  source_label?: string | null;
  spoiler_mode?: SpoilerMode | null;
  preset_id?: string | null;
  frames: GameplayFrameInput[];
}

export interface GameplayReviewAnalysisInput {
  session_id: string;
  max_highlights?: number;
  platforms?: string[];
}

export interface GameplayPresetInput {
  game_id: string;
  display_name: string;
  coaching_focus: string[];
  audio_feedback: boolean;
  spoiler_mode: SpoilerMode;
  notes?: string | null;
}

export interface GameplayReviewPreset extends GameplayPresetInput {
  updated_at_ms: number;
}

export interface GameplayReviewQuestionInput {
  session_id: string;
  question: string;
}

export interface GameplayReviewClipInput {
  session_id: string;
  platform?: string | null;
  highlight_id?: string | null;
}

export interface GameplayHighlight {
  id: string;
  frame_index: number;
  captured_at_ms?: number | null;
  title: string;
  rationale: string;
  confidence: number;
  kind: 'highlight' | 'mistake' | 'coaching';
}

export interface GameplayClipCandidate {
  id: string;
  frame_index: number;
  start_label: string;
  end_label: string;
  rationale: string;
  confidence: number;
}

export interface GameplayPlatformDraft {
  platform: string;
  title: string;
  description: string;
  tags: string[];
}

export interface GameplayReviewAnalysis {
  recap: string;
  highlights: GameplayHighlight[];
  clip_candidates: GameplayClipCandidate[];
  draft_metadata: GameplayPlatformDraft[];
  follow_up_questions: string[];
  spoiler_note?: string | null;
}

export interface GameplayReviewSession {
  session_id: string;
  game_id: string;
  session_title: string;
  source_label?: string | null;
  spoiler_mode: SpoilerMode;
  preset_id?: string | null;
  imported_at_ms: number;
  analyzed_at_ms?: number | null;
  frames: GameplayFrameInput[];
  analysis?: GameplayReviewAnalysis | null;
}

export interface GameplayReviewQuestionResult {
  answer: string;
  matched_highlights: GameplayHighlight[];
  suggested_follow_up: string[];
}

export interface PreparedGameplayFrame extends GameplayFrameInput {
  source_name: string;
}

export async function prepareGameplayFrames(
  files: File[],
  maxFrames = 12
): Promise<PreparedGameplayFrame[]> {
  const imageFiles = files
    .filter(file => file.type.startsWith('image/'))
    .sort((left, right) => {
      const leftName = (left.webkitRelativePath || left.name).toLowerCase();
      const rightName = (right.webkitRelativePath || right.name).toLowerCase();
      return leftName.localeCompare(rightName);
    })
    .slice(0, Math.max(1, maxFrames));

  log(
    'prepareGameplayFrames: selected %d image(s) from %d file(s)',
    imageFiles.length,
    files.length
  );
  const prepared: PreparedGameplayFrame[] = [];
  for (const file of imageFiles) {
    const buffer = await file.arrayBuffer();
    const bytes = new Uint8Array(buffer);
    let base64 = '';
    for (let index = 0; index < bytes.length; index += 0x8000) {
      base64 += String.fromCharCode(...bytes.subarray(index, index + 0x8000));
    }
    prepared.push({
      source_name: file.webkitRelativePath || file.name,
      file_name: file.webkitRelativePath || file.name,
      image_ref: `data:${file.type || 'image/png'};base64,${btoa(base64)}`,
      captured_at_ms: file.lastModified || null,
    });
  }
  return prepared;
}

export async function registerGameplaySession(
  payload: GameplayReviewSessionInput
): Promise<GameplayReviewSession> {
  const result = await callCoreRpc<GameplayReviewSession>({
    method: 'openhuman.gameplay_review_register_session',
    params: payload,
  });
  return result;
}

export async function analyzeGameplaySession(
  payload: GameplayReviewAnalysisInput
): Promise<GameplayReviewSession> {
  const result = await callCoreRpc<GameplayReviewSession>({
    method: 'openhuman.gameplay_review_analyze_session',
    params: payload,
  });
  return result;
}

export async function listGameplaySessions(gameId?: string): Promise<GameplayReviewSession[]> {
  const result = await callCoreRpc<GameplayReviewSession[]>({
    method: 'openhuman.gameplay_review_list_sessions',
    params: gameId ? { game_id: gameId } : {},
  });
  return result;
}

export async function saveGameplayPreset(payload: GameplayPresetInput): Promise<unknown> {
  return callCoreRpc({ method: 'openhuman.gameplay_review_set_preset', params: payload });
}

export async function listGameplayPresets(): Promise<GameplayReviewPreset[]> {
  return callCoreRpc<GameplayReviewPreset[]>({ method: 'openhuman.gameplay_review_list_presets' });
}

export async function askGameplaySession(
  payload: GameplayReviewQuestionInput
): Promise<GameplayReviewQuestionResult> {
  return callCoreRpc<GameplayReviewQuestionResult>({
    method: 'openhuman.gameplay_review_ask_session',
    params: payload,
  });
}

export async function draftGameplayClipMetadata(
  payload: GameplayReviewClipInput
): Promise<GameplayPlatformDraft[]> {
  return callCoreRpc<GameplayPlatformDraft[]>({
    method: 'openhuman.gameplay_review_draft_clip_metadata',
    params: payload,
  });
}

export function flattenHighlights(session: GameplayReviewSession | null): GameplayHighlight[] {
  return session?.analysis?.highlights ?? [];
}

export function flattenDrafts(session: GameplayReviewSession | null): GameplayPlatformDraft[] {
  return session?.analysis?.draft_metadata ?? [];
}

export function flattenClipCandidates(
  session: GameplayReviewSession | null
): GameplayClipCandidate[] {
  return session?.analysis?.clip_candidates ?? [];
}

export function formatSpoilerMode(mode: SpoilerMode): string {
  switch (mode) {
    case 'off':
      return 'gameplay.spoiler.off';
    case 'full':
      return 'gameplay.spoiler.full';
    default:
      return 'gameplay.spoiler.light';
  }
}

export function normalizeGameplayError(error: unknown): string {
  if (error instanceof Error) return error.message;
  if (typeof error === 'string') return error;
  return 'Gameplay review failed';
}
