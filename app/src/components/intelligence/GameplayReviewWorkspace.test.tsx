import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import {
  analyzeGameplaySession,
  askGameplaySession,
  draftGameplayClipMetadata,
  type GameplayReviewAnalysis,
  type GameplayReviewSession,
  listGameplaySessions,
  prepareGameplayFrames,
  registerGameplaySession,
  saveGameplayPreset,
} from '../../services/gameplayReviewService';
import { GameplayReviewWorkspace } from './GameplayReviewWorkspace';

vi.mock('../../services/gameplayReviewService', () => ({
  analyzeGameplaySession: vi.fn(),
  askGameplaySession: vi.fn(),
  draftGameplayClipMetadata: vi.fn(),
  flattenClipCandidates: vi.fn((session: any) => session?.analysis?.clip_candidates ?? []),
  flattenDrafts: vi.fn((session: any) => session?.analysis?.draft_metadata ?? []),
  flattenHighlights: vi.fn((session: any) => session?.analysis?.highlights ?? []),
  formatSpoilerMode: vi.fn((mode: string) =>
    mode === 'full' ? 'Full spoilers' : 'Light spoilers'
  ),
  listGameplaySessions: vi.fn(),
  normalizeGameplayError: vi.fn((error: unknown) =>
    error instanceof Error ? error.message : String(error)
  ),
  prepareGameplayFrames: vi.fn(),
  registerGameplaySession: vi.fn(),
  saveGameplayPreset: vi.fn(),
}));

const mockedListGameplaySessions = vi.mocked(listGameplaySessions);
const mockedPrepareGameplayFrames = vi.mocked(prepareGameplayFrames);
const mockedRegisterGameplaySession = vi.mocked(registerGameplaySession);
const mockedAnalyzeGameplaySession = vi.mocked(analyzeGameplaySession);
const mockedAskGameplaySession = vi.mocked(askGameplaySession);
const mockedDraftGameplayClipMetadata = vi.mocked(draftGameplayClipMetadata);
const mockedSaveGameplayPreset = vi.mocked(saveGameplayPreset);

describe('GameplayReviewWorkspace', () => {
  beforeEach(() => {
    mockedListGameplaySessions.mockResolvedValue([]);
    mockedPrepareGameplayFrames.mockReset();
    mockedRegisterGameplaySession.mockReset();
    mockedAnalyzeGameplaySession.mockReset();
    mockedAskGameplaySession.mockReset();
    mockedDraftGameplayClipMetadata.mockReset();
    mockedSaveGameplayPreset.mockReset();
  });

  it('imports selected frames, analyzes the session, and asks follow-up questions', async () => {
    mockedPrepareGameplayFrames.mockResolvedValue([
      {
        source_name: 'frame-1.png',
        file_name: 'frame-1.png',
        image_ref: 'data:image/png;base64,AAA',
        captured_at_ms: 123,
      },
    ]);
    mockedRegisterGameplaySession.mockResolvedValue({
      session_id: 'gameplay-apex-1',
      game_id: 'Apex Legends',
      session_title: 'Ranked climb',
      source_label: '/recordings/apex',
      spoiler_mode: 'light',
      preset_id: 'Apex preset',
      imported_at_ms: 1000,
      analyzed_at_ms: null,
      frames: [
        { file_name: 'frame-1.png', image_ref: 'data:image/png;base64,AAA', captured_at_ms: 123 },
      ],
      analysis: null,
    });
    mockedAnalyzeGameplaySession.mockResolvedValue({
      session_id: 'gameplay-apex-1',
      game_id: 'Apex Legends',
      session_title: 'Ranked climb',
      source_label: '/recordings/apex',
      spoiler_mode: 'light',
      preset_id: 'Apex preset',
      imported_at_ms: 1000,
      analyzed_at_ms: 2000,
      frames: [
        { file_name: 'frame-1.png', image_ref: 'data:image/png;base64,AAA', captured_at_ms: 123 },
      ],
      analysis: {
        recap: 'Gameplay recap',
        highlights: [
          {
            id: 'h1',
            frame_index: 0,
            captured_at_ms: 123,
            title: 'Highlight: clutch fight',
            rationale: 'Clean finish',
            confidence: 0.93,
            kind: 'highlight',
          },
        ],
        clip_candidates: [
          {
            id: 'c1',
            frame_index: 0,
            start_label: 'frame-1.png',
            end_label: 'frame-1.png',
            rationale: 'Clean finish',
            confidence: 0.93,
          },
        ],
        draft_metadata: [
          {
            platform: 'twitch',
            title: 'Apex Legends — Highlight: clutch fight (twitch)',
            description: 'Session: Ranked climb',
            tags: ['apex-legends'],
          },
        ],
        follow_up_questions: ['What was the turning point?'],
        spoiler_note: null,
      },
    });
    mockedAskGameplaySession.mockResolvedValue({
      answer: 'Best clip candidate for Ranked climb: Highlight: clutch fight — Clean finish',
      matched_highlights: [],
      suggested_follow_up: ['What was the turning point?'],
    });
    mockedDraftGameplayClipMetadata.mockResolvedValue([
      {
        platform: 'twitch',
        title: 'Apex Legends — Highlight: clutch fight (twitch)',
        description: 'Session: Ranked climb',
        tags: ['apex-legends'],
      },
    ]);
    mockedSaveGameplayPreset.mockResolvedValue({});

    const { container } = render(<GameplayReviewWorkspace />);

    await waitFor(() => expect(screen.getByText('No saved sessions yet.')).toBeInTheDocument());

    fireEvent.change(screen.getByPlaceholderText('Apex Legends'), {
      target: { value: 'Apex Legends' },
    });
    fireEvent.change(screen.getByPlaceholderText('Ranked climb on Friday night'), {
      target: { value: 'Ranked climb' },
    });
    fireEvent.change(screen.getByPlaceholderText('/recordings/apex/night-01'), {
      target: { value: '/recordings/apex' },
    });
    fireEvent.change(screen.getByPlaceholderText('Apex coaching preset'), {
      target: { value: 'Apex preset' },
    });
    fireEvent.change(screen.getByPlaceholderText('aim, positioning, fight selection'), {
      target: { value: 'aim, positioning' },
    });
    fireEvent.change(screen.getByPlaceholderText('What should the reviewer pay attention to?'), {
      target: { value: 'Play clean and keep it spoiler-safe.' },
    });
    fireEvent.change(screen.getByPlaceholderText('twitch,kick,youtube'), {
      target: { value: 'twitch,kick' },
    });

    const fileInput = container.querySelector('input[type="file"]') as HTMLInputElement | null;
    expect(fileInput).not.toBeNull();
    const file = new File([new Uint8Array([1, 2, 3])], 'frame-1.png', { type: 'image/png' });
    Object.defineProperty(file, 'webkitRelativePath', { value: 'session/frame-1.png' });
    fireEvent.change(fileInput as HTMLInputElement, { target: { files: [file] } });

    fireEvent.click(screen.getByRole('button', { name: /import and analyze/i }));

    await waitFor(() => expect(mockedPrepareGameplayFrames).toHaveBeenCalled());
    await waitFor(() => expect(screen.getByText(/Gameplay recap/)).toBeInTheDocument());
    expect(
      screen.getByText('Highlight: clutch fight', { selector: 'div.font-medium.text-stone-900' })
    ).toBeInTheDocument();

    fireEvent.click(screen.getByRole('button', { name: /ask session/i }));
    await waitFor(() => expect(mockedAskGameplaySession).toHaveBeenCalled());
    expect(screen.getByText(/Best clip candidate/)).toBeInTheDocument();
  });

  const BASE_ANALYSIS: GameplayReviewAnalysis = {
    recap: 'Solid session recap',
    highlights: [
      {
        id: 'h1',
        frame_index: 0,
        captured_at_ms: 123,
        title: 'Clutch fight',
        rationale: 'Clean finish',
        confidence: 0.9,
        kind: 'highlight',
      },
    ],
    clip_candidates: [
      {
        id: 'c1',
        frame_index: 0,
        start_label: 'frame-1.png',
        end_label: 'frame-2.png',
        rationale: 'Great pacing',
        confidence: 0.8,
      },
    ],
    draft_metadata: [
      { platform: 'twitch', title: 'Clip title', description: 'Clip desc', tags: ['apex'] },
    ],
    follow_up_questions: ['What was the turning point?'],
    spoiler_note: 'Endgame results hidden.',
  };

  function analyzedSession(overrides: Partial<GameplayReviewSession> = {}): GameplayReviewSession {
    return {
      session_id: 'session-1',
      game_id: 'Apex Legends',
      session_title: 'Ranked climb',
      source_label: null,
      spoiler_mode: 'full',
      preset_id: null,
      imported_at_ms: 1000,
      analyzed_at_ms: 2000,
      frames: [
        { file_name: 'frame-1.png', image_ref: 'data:image/png;base64,AAA', captured_at_ms: 123 },
      ],
      analysis: BASE_ANALYSIS,
      ...overrides,
    };
  }

  function selectFile(container: HTMLElement) {
    const fileInput = container.querySelector('input[type="file"]') as HTMLInputElement;
    const file = new File([new Uint8Array([1, 2, 3])], 'frame-1.png', { type: 'image/png' });
    fireEvent.change(fileInput, { target: { files: [file] } });
  }

  it('surfaces an error when the initial session fetch fails', async () => {
    mockedListGameplaySessions.mockRejectedValueOnce(new Error('list failed'));

    render(<GameplayReviewWorkspace />);

    await waitFor(() => expect(screen.getByText('list failed')).toBeInTheDocument());
  });

  it('validates required fields before importing', async () => {
    render(<GameplayReviewWorkspace />);
    await waitFor(() => expect(screen.getByText('No saved sessions yet.')).toBeInTheDocument());

    const importButton = screen.getByRole('button', { name: /import and analyze/i });

    fireEvent.click(importButton);
    expect(screen.getByText('Game name is required.')).toBeInTheDocument();

    fireEvent.change(screen.getByPlaceholderText('Apex Legends'), { target: { value: 'Apex' } });
    fireEvent.click(importButton);
    expect(screen.getByText('Session title is required.')).toBeInTheDocument();

    fireEvent.change(screen.getByPlaceholderText('Ranked climb on Friday night'), {
      target: { value: 'Ranked' },
    });
    fireEvent.click(importButton);
    expect(
      screen.getByText('Choose a folder or a set of keyframe images first.')
    ).toBeInTheDocument();
    expect(mockedPrepareGameplayFrames).not.toHaveBeenCalled();
  });

  it('errors when no image frames are found and skips preset save without a preset name', async () => {
    mockedPrepareGameplayFrames.mockResolvedValue([]);

    const { container } = render(<GameplayReviewWorkspace />);
    await waitFor(() => expect(screen.getByText('No saved sessions yet.')).toBeInTheDocument());

    fireEvent.change(screen.getByPlaceholderText('Apex Legends'), { target: { value: 'Apex' } });
    fireEvent.change(screen.getByPlaceholderText('Ranked climb on Friday night'), {
      target: { value: 'Ranked' },
    });
    selectFile(container);

    fireEvent.click(screen.getByRole('button', { name: /import and analyze/i }));

    await waitFor(() =>
      expect(screen.getByText('No image frames were found in that folder.')).toBeInTheDocument()
    );
    expect(mockedSaveGameplayPreset).not.toHaveBeenCalled();
    expect(mockedRegisterGameplaySession).not.toHaveBeenCalled();
  });

  it('reports a toast and inline error when registration fails', async () => {
    mockedPrepareGameplayFrames.mockResolvedValue([
      {
        source_name: 'frame-1.png',
        file_name: 'frame-1.png',
        image_ref: 'data:image/png;base64,AAA',
        captured_at_ms: 123,
      },
    ]);
    mockedRegisterGameplaySession.mockRejectedValue(new Error('register boom'));
    const onToast = vi.fn();

    const { container } = render(<GameplayReviewWorkspace onToast={onToast} />);
    await waitFor(() => expect(screen.getByText('No saved sessions yet.')).toBeInTheDocument());

    fireEvent.change(screen.getByPlaceholderText('Apex Legends'), { target: { value: 'Apex' } });
    fireEvent.change(screen.getByPlaceholderText('Ranked climb on Friday night'), {
      target: { value: 'Ranked' },
    });
    selectFile(container);

    fireEvent.click(screen.getByRole('button', { name: /import and analyze/i }));

    await waitFor(() => expect(screen.getByText('register boom')).toBeInTheDocument());
    expect(onToast).toHaveBeenCalledWith(
      expect.objectContaining({ type: 'error', title: 'Gameplay review failed' })
    );
  });

  it('renders an analyzed session from history with highlights, clips, drafts, and follow-ups', async () => {
    mockedListGameplaySessions.mockResolvedValue([analyzedSession()]);

    render(<GameplayReviewWorkspace />);

    await waitFor(() => expect(screen.getByText('Solid session recap')).toBeInTheDocument());
    expect(screen.getByText('Endgame results hidden.')).toBeInTheDocument();
    expect(screen.getByText('Clean finish')).toBeInTheDocument();
    expect(screen.getByText(/through frame-2.png/)).toBeInTheDocument();
    expect(screen.getByText('Clip title')).toBeInTheDocument();
    expect(screen.getByText('#apex')).toBeInTheDocument();

    // Clicking a follow-up question seeds the question input.
    fireEvent.click(screen.getByRole('button', { name: 'What was the turning point?' }));
    expect(
      (screen.getByPlaceholderText('What were my best fights?') as HTMLInputElement).value
    ).toBe('What was the turning point?');

    // Refresh history re-fetches sessions.
    fireEvent.click(screen.getByRole('button', { name: /refresh history/i }));
    await waitFor(() => expect(mockedListGameplaySessions).toHaveBeenCalledTimes(2));
  });

  it('shows empty states when the active session has no analysis', async () => {
    mockedListGameplaySessions.mockResolvedValue([analyzedSession({ analysis: null })]);

    render(<GameplayReviewWorkspace />);

    await waitFor(() =>
      expect(screen.getByText('No highlights detected yet.')).toBeInTheDocument()
    );
    expect(screen.getByText('No clip candidates yet.')).toBeInTheDocument();
    expect(screen.getByText('Draft metadata will appear after analysis.')).toBeInTheDocument();
  });

  it('refreshes clip metadata for the active session', async () => {
    mockedListGameplaySessions.mockResolvedValue([analyzedSession()]);
    mockedDraftGameplayClipMetadata.mockResolvedValue([
      { platform: 'kick', title: 'Refreshed title', description: 'New desc', tags: ['fresh'] },
    ]);

    render(<GameplayReviewWorkspace />);
    await waitFor(() => expect(screen.getByText('Clip title')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: /refresh clip metadata/i }));

    await waitFor(() => expect(screen.getByText('Refreshed title')).toBeInTheDocument());
    expect(mockedDraftGameplayClipMetadata).toHaveBeenCalledWith(
      expect.objectContaining({ session_id: 'session-1', highlight_id: 'h1' })
    );
  });

  it('surfaces an error when refreshing clip metadata fails', async () => {
    mockedListGameplaySessions.mockResolvedValue([analyzedSession()]);
    mockedDraftGameplayClipMetadata.mockRejectedValue(new Error('draft boom'));

    render(<GameplayReviewWorkspace />);
    await waitFor(() => expect(screen.getByText('Clip title')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: /refresh clip metadata/i }));

    await waitFor(() => expect(screen.getByText('draft boom')).toBeInTheDocument());
  });

  it('surfaces an error when asking the session fails', async () => {
    mockedListGameplaySessions.mockResolvedValue([analyzedSession()]);
    mockedAskGameplaySession.mockRejectedValue(new Error('ask boom'));

    render(<GameplayReviewWorkspace />);
    await waitFor(() => expect(screen.getByText('Solid session recap')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: /ask session/i }));

    await waitFor(() => expect(screen.getByText('ask boom')).toBeInTheDocument());
  });

  it('updates spoiler mode, audio feedback, and the question input', async () => {
    mockedListGameplaySessions.mockResolvedValue([analyzedSession()]);

    const { container } = render(<GameplayReviewWorkspace />);
    await waitFor(() => expect(screen.getByText('Solid session recap')).toBeInTheDocument());

    const spoilerSelect = container.querySelector('select') as HTMLSelectElement;
    fireEvent.change(spoilerSelect, { target: { value: 'off' } });
    expect(spoilerSelect.value).toBe('off');

    const audioCheckbox = container.querySelector('input[type="checkbox"]') as HTMLInputElement;
    fireEvent.click(audioCheckbox);
    expect(audioCheckbox.checked).toBe(true);

    const questionInput = screen.getByPlaceholderText(
      'What were my best fights?'
    ) as HTMLInputElement;
    fireEvent.change(questionInput, { target: { value: 'Where did I lose tempo?' } });
    expect(questionInput.value).toBe('Where did I lose tempo?');

    // The hidden file picker is triggered via the visible button.
    fireEvent.click(screen.getByRole('button', { name: /choose folder \/ frames/i }));
  });

  it('switches the active session when picking one from history', async () => {
    const first = analyzedSession();
    const second = analyzedSession({
      session_id: 'session-2',
      session_title: 'Second session',
      analysis: { ...BASE_ANALYSIS, recap: 'Second recap' },
    });
    mockedListGameplaySessions.mockResolvedValue([first, second]);

    render(<GameplayReviewWorkspace />);
    await waitFor(() => expect(screen.getByText('Solid session recap')).toBeInTheDocument());

    fireEvent.click(screen.getByRole('button', { name: /Second session/ }));

    await waitFor(() => expect(screen.getByText('Second recap')).toBeInTheDocument());
  });
});
