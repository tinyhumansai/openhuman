import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  analyzeGameplaySession,
  askGameplaySession,
  draftGameplayClipMetadata,
  flattenClipCandidates,
  flattenDrafts,
  flattenHighlights,
  formatSpoilerMode,
  type GameplayReviewAnalysis,
  type GameplayReviewSession,
  listGameplayPresets,
  listGameplaySessions,
  normalizeGameplayError,
  prepareGameplayFrames,
  registerGameplaySession,
  saveGameplayPreset,
} from './gameplayReviewService';

const mockCallCoreRpc = vi.fn();

vi.mock('./coreRpcClient', () => ({
  callCoreRpc: (...args: unknown[]) => mockCallCoreRpc(...args),
}));

function makeImageFile(
  name: string,
  bytes: number[],
  options: { type?: string; relativePath?: string; lastModified?: number } = {}
): File {
  const file = new File([new Uint8Array(bytes)], name, {
    type: options.type ?? 'image/png',
    lastModified: options.lastModified ?? 0,
  });
  if (options.relativePath !== undefined) {
    Object.defineProperty(file, 'webkitRelativePath', { value: options.relativePath });
  }
  return file;
}

const ANALYSIS: GameplayReviewAnalysis = {
  recap: 'Recap',
  highlights: [
    {
      id: 'h1',
      frame_index: 0,
      captured_at_ms: 1,
      title: 'Clutch',
      rationale: 'Clean',
      confidence: 0.9,
      kind: 'highlight',
    },
  ],
  clip_candidates: [
    {
      id: 'c1',
      frame_index: 0,
      start_label: 'a',
      end_label: 'b',
      rationale: 'why',
      confidence: 0.8,
    },
  ],
  draft_metadata: [{ platform: 'twitch', title: 't', description: 'd', tags: ['x'] }],
  follow_up_questions: ['q?'],
  spoiler_note: null,
};

const SESSION: GameplayReviewSession = {
  session_id: 's1',
  game_id: 'Apex',
  session_title: 'Ranked',
  source_label: null,
  spoiler_mode: 'light',
  preset_id: null,
  imported_at_ms: 1000,
  analyzed_at_ms: 2000,
  frames: [],
  analysis: ANALYSIS,
};

describe('gameplayReviewService', () => {
  beforeEach(() => {
    mockCallCoreRpc.mockReset();
  });

  describe('prepareGameplayFrames', () => {
    it('filters to images, sorts by lowercased path, caps to maxFrames, and base64-encodes', async () => {
      const files = [
        makeImageFile('B.png', [1, 2, 3], { type: 'image/png', lastModified: 42 }),
        makeImageFile('a.png', [4, 5], { type: 'image/png', lastModified: 7 }),
        // Non-image file should be filtered out.
        new File([new Uint8Array([9])], 'notes.txt', { type: 'text/plain' }),
      ];

      const prepared = await prepareGameplayFrames(files, 5);

      expect(prepared).toHaveLength(2);
      // 'a.png' sorts before 'b.png' (case-insensitive).
      expect(prepared[0].file_name).toBe('a.png');
      expect(prepared[1].file_name).toBe('B.png');
      expect(prepared[0].source_name).toBe('a.png');
      expect(prepared[0].image_ref.startsWith('data:image/png;base64,')).toBe(true);
      expect(prepared[0].captured_at_ms).toBe(7);
      // Decoding the payload yields the original bytes.
      const payload = prepared[0].image_ref.split(',')[1];
      const decoded = atob(payload);
      expect([decoded.charCodeAt(0), decoded.charCodeAt(1)]).toEqual([4, 5]);
    });

    it('prefers webkitRelativePath and maps lastModified=0 to null', async () => {
      const file = makeImageFile('frame.png', [10], {
        type: 'image/jpeg',
        relativePath: 'session/frame.png',
        lastModified: 0,
      });

      const [prepared] = await prepareGameplayFrames([file]);

      expect(prepared.file_name).toBe('session/frame.png');
      expect(prepared.source_name).toBe('session/frame.png');
      expect(prepared.image_ref.startsWith('data:image/jpeg;base64,')).toBe(true);
      expect(prepared.captured_at_ms).toBeNull();
    });

    it('honors maxFrames by slicing the sorted image list', async () => {
      const files = [
        makeImageFile('1.png', [1]),
        makeImageFile('2.png', [2]),
        makeImageFile('3.png', [3]),
      ];

      const prepared = await prepareGameplayFrames(files, 2);

      expect(prepared.map(frame => frame.file_name)).toEqual(['1.png', '2.png']);
    });

    it('floors maxFrames at 1 even when callers pass 0', async () => {
      const prepared = await prepareGameplayFrames([makeImageFile('only.png', [1])], 0);

      expect(prepared).toHaveLength(1);
    });
  });

  describe('RPC wrappers', () => {
    it('registerGameplaySession forwards the payload', async () => {
      mockCallCoreRpc.mockResolvedValueOnce(SESSION);
      const payload = { game_id: 'Apex', session_title: 'Ranked', frames: [] };

      const result = await registerGameplaySession(payload);

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.gameplay_review_register_session',
        params: payload,
      });
      expect(result).toBe(SESSION);
    });

    it('analyzeGameplaySession forwards the payload', async () => {
      mockCallCoreRpc.mockResolvedValueOnce(SESSION);
      const payload = { session_id: 's1', max_highlights: 5, platforms: ['twitch'] };

      const result = await analyzeGameplaySession(payload);

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.gameplay_review_analyze_session',
        params: payload,
      });
      expect(result).toBe(SESSION);
    });

    it('listGameplaySessions sends game_id filter when provided', async () => {
      mockCallCoreRpc.mockResolvedValueOnce([SESSION]);

      const result = await listGameplaySessions('Apex');

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.gameplay_review_list_sessions',
        params: { game_id: 'Apex' },
      });
      expect(result).toEqual([SESSION]);
    });

    it('listGameplaySessions sends empty params when no game id', async () => {
      mockCallCoreRpc.mockResolvedValueOnce([]);

      await listGameplaySessions();

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.gameplay_review_list_sessions',
        params: {},
      });
    });

    it('saveGameplayPreset forwards the payload', async () => {
      mockCallCoreRpc.mockResolvedValueOnce({});
      const payload = {
        game_id: 'Apex',
        display_name: 'Preset',
        coaching_focus: ['aim'],
        audio_feedback: true,
        spoiler_mode: 'full' as const,
        notes: null,
      };

      await saveGameplayPreset(payload);

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.gameplay_review_set_preset',
        params: payload,
      });
    });

    it('listGameplayPresets calls the presets RPC', async () => {
      mockCallCoreRpc.mockResolvedValueOnce([]);

      const result = await listGameplayPresets();

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.gameplay_review_list_presets',
      });
      expect(result).toEqual([]);
    });

    it('askGameplaySession forwards the question payload', async () => {
      const answer = { answer: 'A', matched_highlights: [], suggested_follow_up: [] };
      mockCallCoreRpc.mockResolvedValueOnce(answer);
      const payload = { session_id: 's1', question: 'best?' };

      const result = await askGameplaySession(payload);

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.gameplay_review_ask_session',
        params: payload,
      });
      expect(result).toBe(answer);
    });

    it('draftGameplayClipMetadata forwards the payload', async () => {
      const drafts = [{ platform: 'twitch', title: 't', description: 'd', tags: [] }];
      mockCallCoreRpc.mockResolvedValueOnce(drafts);
      const payload = { session_id: 's1', platform: 'twitch', highlight_id: 'h1' };

      const result = await draftGameplayClipMetadata(payload);

      expect(mockCallCoreRpc).toHaveBeenCalledWith({
        method: 'openhuman.gameplay_review_draft_clip_metadata',
        params: payload,
      });
      expect(result).toBe(drafts);
    });
  });

  describe('flatten helpers', () => {
    it('flattenHighlights returns analysis highlights or [] when missing', () => {
      expect(flattenHighlights(SESSION)).toBe(ANALYSIS.highlights);
      expect(flattenHighlights(null)).toEqual([]);
      expect(flattenHighlights({ ...SESSION, analysis: null })).toEqual([]);
    });

    it('flattenDrafts returns analysis drafts or [] when missing', () => {
      expect(flattenDrafts(SESSION)).toBe(ANALYSIS.draft_metadata);
      expect(flattenDrafts(null)).toEqual([]);
      expect(flattenDrafts({ ...SESSION, analysis: null })).toEqual([]);
    });

    it('flattenClipCandidates returns analysis clips or [] when missing', () => {
      expect(flattenClipCandidates(SESSION)).toBe(ANALYSIS.clip_candidates);
      expect(flattenClipCandidates(null)).toEqual([]);
      expect(flattenClipCandidates({ ...SESSION, analysis: null })).toEqual([]);
    });
  });

  describe('formatSpoilerMode', () => {
    it('maps each spoiler mode to its label', () => {
      expect(formatSpoilerMode('off')).toBe('gameplay.spoiler.off');
      expect(formatSpoilerMode('full')).toBe('gameplay.spoiler.full');
      expect(formatSpoilerMode('light')).toBe('gameplay.spoiler.light');
    });
  });

  describe('normalizeGameplayError', () => {
    it('returns the message for Error instances', () => {
      expect(normalizeGameplayError(new Error('boom'))).toBe('boom');
    });

    it('returns the string for string errors', () => {
      expect(normalizeGameplayError('plain failure')).toBe('plain failure');
    });

    it('returns a default message for unknown error shapes', () => {
      expect(normalizeGameplayError({ weird: true })).toBe('Gameplay review failed');
    });
  });
});
