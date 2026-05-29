import { useCallback, useEffect, useMemo, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import {
  analyzeGameplaySession,
  askGameplaySession,
  draftGameplayClipMetadata,
  flattenClipCandidates,
  flattenDrafts,
  flattenHighlights,
  formatSpoilerMode,
  type GameplayFrameInput,
  type GameplayReviewSession,
  listGameplaySessions,
  normalizeGameplayError,
  prepareGameplayFrames,
  registerGameplaySession,
  saveGameplayPreset,
  type SpoilerMode,
} from '../../services/gameplayReviewService';
import type { ToastNotification } from '../../types/intelligence';

interface GameplayReviewWorkspaceProps {
  onToast?: (toast: Omit<ToastNotification, 'id'>) => void;
}

interface ImportState {
  gameId: string;
  sessionTitle: string;
  sourceLabel: string;
  spoilerMode: SpoilerMode;
  presetName: string;
  coachingFocus: string;
  notes: string;
  audioFeedback: boolean;
  platforms: string;
}

const INITIAL_IMPORT_STATE: ImportState = {
  gameId: '',
  sessionTitle: '',
  sourceLabel: '',
  spoilerMode: 'light',
  presetName: '',
  coachingFocus: '',
  notes: '',
  audioFeedback: false,
  platforms: 'twitch,kick,youtube',
};

const DIRECTORY_INPUT_PROPS: Record<string, string> = { webkitdirectory: '' };

function formatTimestamp(ms?: number | null): string {
  if (!ms) return 'n/a';
  return new Date(ms).toLocaleString();
}

function splitPlatforms(value: string): string[] {
  return value
    .split(',')
    .map(item => item.trim())
    .filter(Boolean);
}

export function GameplayReviewWorkspace({ onToast }: GameplayReviewWorkspaceProps) {
  const { t } = useT();
  const [form, setForm] = useState(INITIAL_IMPORT_STATE);
  const [selectedFiles, setSelectedFiles] = useState<File[]>([]);
  const [recentSessions, setRecentSessions] = useState<GameplayReviewSession[]>([]);
  const [activeSession, setActiveSession] = useState<GameplayReviewSession | null>(null);
  const [question, setQuestion] = useState('What were my best moments?');
  const [questionAnswer, setQuestionAnswer] = useState<string>('');
  const [questionBusy, setQuestionBusy] = useState(false);
  const [importBusy, setImportBusy] = useState(false);
  const [analysisBusy, setAnalysisBusy] = useState(false);
  const [draftBusy, setDraftBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const fileInputRef = useRef<HTMLInputElement | null>(null);

  const refreshSessions = useCallback(async () => {
    try {
      const sessions = await listGameplaySessions();
      setRecentSessions(sessions);
      setActiveSession(currentSession => currentSession ?? sessions[0] ?? null);
    } catch (err) {
      setError(normalizeGameplayError(err));
    }
  }, []);

  useEffect(() => {
    void refreshSessions();
  }, [refreshSessions]);

  const activeHighlights = useMemo(() => flattenHighlights(activeSession), [activeSession]);
  const activeDrafts = useMemo(() => flattenDrafts(activeSession), [activeSession]);
  const activeClips = useMemo(() => flattenClipCandidates(activeSession), [activeSession]);

  const handleSelectFiles = useCallback(() => {
    fileInputRef.current?.click();
  }, []);

  const handleFilesChanged = useCallback((event: React.ChangeEvent<HTMLInputElement>) => {
    const files = Array.from(event.target.files ?? []);
    setSelectedFiles(files);
  }, []);

  const handleImportSession = useCallback(async () => {
    setError(null);
    if (!form.gameId.trim()) {
      setError('Game name is required.');
      return;
    }
    if (!form.sessionTitle.trim()) {
      setError('Session title is required.');
      return;
    }
    if (selectedFiles.length === 0) {
      setError('Choose a folder or a set of keyframe images first.');
      return;
    }

    setImportBusy(true);
    try {
      const frames = await prepareGameplayFrames(selectedFiles);
      if (frames.length === 0) {
        throw new Error('No image frames were found in that folder.');
      }

      if (form.presetName.trim()) {
        await saveGameplayPreset({
          game_id: form.gameId.trim(),
          display_name: form.presetName.trim(),
          coaching_focus: form.coachingFocus
            .split(',')
            .map(item => item.trim())
            .filter(Boolean),
          audio_feedback: form.audioFeedback,
          spoiler_mode: form.spoilerMode,
          notes: form.notes.trim() || null,
        });
      }

      const session = await registerGameplaySession({
        game_id: form.gameId.trim(),
        session_title: form.sessionTitle.trim(),
        source_label: form.sourceLabel.trim() || null,
        spoiler_mode: form.spoilerMode,
        preset_id: form.gameId.trim() || null,
        frames: frames.map(
          frame =>
            ({
              file_name: frame.file_name,
              image_ref: frame.image_ref,
              captured_at_ms: frame.captured_at_ms,
            }) satisfies GameplayFrameInput
        ),
      });

      setAnalysisBusy(true);
      const analyzed = await analyzeGameplaySession({
        session_id: session.session_id,
        max_highlights: 5,
        platforms: splitPlatforms(form.platforms),
      });
      setActiveSession(analyzed);
      setQuestionAnswer('');
      onToast?.({
        type: 'success',
        title: 'Gameplay session analyzed',
        message: `Reviewed ${analyzed.frames.length} frame(s) for ${analyzed.game_id}.`,
      });
      void refreshSessions();
    } catch (err) {
      const message = normalizeGameplayError(err);
      setError(message);
      onToast?.({ type: 'error', title: 'Gameplay review failed', message });
    } finally {
      setAnalysisBusy(false);
      setImportBusy(false);
    }
  }, [form, onToast, refreshSessions, selectedFiles]);

  const handleQuestion = useCallback(async () => {
    if (!activeSession) {
      setError('Select or analyze a session first.');
      return;
    }
    setQuestionBusy(true);
    try {
      const response = await askGameplaySession({ session_id: activeSession.session_id, question });
      setQuestionAnswer(response.answer);
    } catch (err) {
      setError(normalizeGameplayError(err));
    } finally {
      setQuestionBusy(false);
    }
  }, [activeSession, question]);

  const handleDraftClips = useCallback(async () => {
    if (!activeSession) return;
    setDraftBusy(true);
    try {
      const drafts = await draftGameplayClipMetadata({
        session_id: activeSession.session_id,
        platform: splitPlatforms(form.platforms)[0] || 'twitch',
        highlight_id: activeHighlights[0]?.id ?? null,
      });
      setActiveSession(prev =>
        prev
          ? {
              ...prev,
              analysis: prev.analysis
                ? { ...prev.analysis, draft_metadata: drafts }
                : prev.analysis,
            }
          : prev
      );
    } catch (err) {
      setError(normalizeGameplayError(err));
    } finally {
      setDraftBusy(false);
    }
  }, [activeHighlights, activeSession, form.platforms]);

  return (
    <div className="space-y-4" data-testid="gameplay-review-workspace">
      <div className="rounded-2xl border border-sage-200 bg-gradient-to-br from-white via-sage-50 to-amber-50 p-5 shadow-soft">
        <div className="flex flex-wrap items-start justify-between gap-4">
          <div className="space-y-2">
            <p className="text-xs font-semibold uppercase tracking-[0.24em] text-sage-700">
              Gameplay review
            </p>
            <h2 className="text-2xl font-bold text-stone-900">
              Import a session, find the clips, draft the post
            </h2>
            <p className="max-w-2xl text-sm text-stone-600">
              Load a folder of keyframes from a long session, generate a concise recap, ask
              follow-up questions, and turn the strongest moments into platform-ready clip metadata.
            </p>
          </div>
          <div className="rounded-xl border border-sage-200 bg-white/80 px-4 py-3 text-sm text-stone-700 shadow-sm">
            <div className="font-semibold text-stone-900">Selected files</div>
            <div>{selectedFiles.length} file(s)</div>
            <div>{t(formatSpoilerMode(form.spoilerMode))}</div>
          </div>
        </div>

        <div className="mt-4 grid gap-3 md:grid-cols-2 xl:grid-cols-3">
          <label className="space-y-1 text-sm">
            <span className="font-medium text-stone-700">Game</span>
            <input
              value={form.gameId}
              onChange={event => setForm(prev => ({ ...prev, gameId: event.target.value }))}
              className="w-full rounded-xl border border-stone-200 bg-white px-3 py-2 outline-none ring-0 transition focus:border-primary-500"
              placeholder="Apex Legends"
            />
          </label>
          <label className="space-y-1 text-sm">
            <span className="font-medium text-stone-700">Session title</span>
            <input
              value={form.sessionTitle}
              onChange={event => setForm(prev => ({ ...prev, sessionTitle: event.target.value }))}
              className="w-full rounded-xl border border-stone-200 bg-white px-3 py-2 outline-none ring-0 transition focus:border-primary-500"
              placeholder="Ranked climb on Friday night"
            />
          </label>
          <label className="space-y-1 text-sm">
            <span className="font-medium text-stone-700">Source label</span>
            <input
              value={form.sourceLabel}
              onChange={event => setForm(prev => ({ ...prev, sourceLabel: event.target.value }))}
              className="w-full rounded-xl border border-stone-200 bg-white px-3 py-2 outline-none ring-0 transition focus:border-primary-500"
              placeholder="/recordings/apex/night-01"
            />
          </label>
          <label className="space-y-1 text-sm">
            <span className="font-medium text-stone-700">Spoiler mode</span>
            <select
              value={form.spoilerMode}
              onChange={event =>
                setForm(prev => ({ ...prev, spoilerMode: event.target.value as SpoilerMode }))
              }
              className="w-full rounded-xl border border-stone-200 bg-white px-3 py-2 outline-none ring-0 transition focus:border-primary-500">
              <option value="off">Off</option>
              <option value="light">Light</option>
              <option value="full">Full</option>
            </select>
          </label>
          <label className="space-y-1 text-sm">
            <span className="font-medium text-stone-700">Preset name</span>
            <input
              value={form.presetName}
              onChange={event => setForm(prev => ({ ...prev, presetName: event.target.value }))}
              className="w-full rounded-xl border border-stone-200 bg-white px-3 py-2 outline-none ring-0 transition focus:border-primary-500"
              placeholder="Apex coaching preset"
            />
          </label>
          <label className="space-y-1 text-sm">
            <span className="font-medium text-stone-700">Focus areas</span>
            <input
              value={form.coachingFocus}
              onChange={event => setForm(prev => ({ ...prev, coachingFocus: event.target.value }))}
              className="w-full rounded-xl border border-stone-200 bg-white px-3 py-2 outline-none ring-0 transition focus:border-primary-500"
              placeholder="aim, positioning, fight selection"
            />
          </label>
          <label className="space-y-1 text-sm md:col-span-2 xl:col-span-3">
            <span className="font-medium text-stone-700">Preset notes</span>
            <textarea
              value={form.notes}
              onChange={event => setForm(prev => ({ ...prev, notes: event.target.value }))}
              className="min-h-24 w-full rounded-xl border border-stone-200 bg-white px-3 py-2 outline-none ring-0 transition focus:border-primary-500"
              placeholder="What should the reviewer pay attention to?"
            />
          </label>
          <label className="space-y-1 text-sm md:col-span-2">
            <span className="font-medium text-stone-700">Clip platforms</span>
            <input
              value={form.platforms}
              onChange={event => setForm(prev => ({ ...prev, platforms: event.target.value }))}
              className="w-full rounded-xl border border-stone-200 bg-white px-3 py-2 outline-none ring-0 transition focus:border-primary-500"
              placeholder="twitch,kick,youtube"
            />
          </label>
          <label className="flex items-center gap-2 self-end text-sm text-stone-700 md:col-span-1">
            <input
              type="checkbox"
              checked={form.audioFeedback}
              onChange={event =>
                setForm(prev => ({ ...prev, audioFeedback: event.target.checked }))
              }
            />
            Enable audio summary notes
          </label>
        </div>

        <div className="mt-4 flex flex-wrap items-center gap-3">
          <input
            ref={fileInputRef}
            type="file"
            multiple
            accept="image/*"
            {...DIRECTORY_INPUT_PROPS}
            className="hidden"
            onChange={handleFilesChanged}
          />
          <button
            type="button"
            onClick={handleSelectFiles}
            className="rounded-xl border border-sage-200 bg-white px-4 py-2 text-sm font-semibold text-sage-800 shadow-sm transition hover:bg-sage-50">
            Choose folder / frames
          </button>
          <button
            type="button"
            onClick={handleImportSession}
            disabled={importBusy || analysisBusy}
            className="rounded-xl bg-primary-600 px-4 py-2 text-sm font-semibold text-white shadow-sm transition hover:bg-primary-700 disabled:cursor-not-allowed disabled:opacity-60">
            {analysisBusy ? 'Analyzing…' : importBusy ? 'Importing…' : 'Import and analyze'}
          </button>
          <button
            type="button"
            onClick={handleDraftClips}
            disabled={draftBusy || !activeSession}
            className="rounded-xl border border-amber-200 bg-white px-4 py-2 text-sm font-semibold text-amber-800 shadow-sm transition hover:bg-amber-50 disabled:cursor-not-allowed disabled:opacity-60">
            {draftBusy ? 'Drafting…' : 'Refresh clip metadata'}
          </button>
          <div className="text-xs text-stone-500">
            The core will analyze image frames. For raw video files, point this at extracted
            keyframes.
          </div>
        </div>
      </div>

      {error && (
        <div className="rounded-xl border border-coral-200 bg-coral-50 px-4 py-3 text-sm text-coral-800">
          {error}
        </div>
      )}

      <div className="grid gap-4 lg:grid-cols-[1.2fr_0.8fr]">
        <section className="space-y-4 rounded-2xl border border-stone-200 bg-white p-5 shadow-soft">
          <div className="flex items-center justify-between gap-2">
            <div>
              <h3 className="text-lg font-semibold text-stone-900">Session viewer</h3>
              <p className="text-sm text-stone-500">
                Recap, highlights, and clip candidates for the current session.
              </p>
            </div>
            <div className="rounded-full border border-stone-200 bg-stone-50 px-3 py-1 text-xs text-stone-600">
              {activeSession ? activeSession.game_id : 'No session selected'}
            </div>
          </div>

          {activeSession ? (
            <div className="space-y-4">
              <div className="rounded-xl border border-sage-200 bg-sage-50 p-4">
                <div className="flex flex-wrap items-center justify-between gap-2">
                  <div>
                    <div className="text-sm font-semibold text-sage-900">
                      {activeSession.session_title}
                    </div>
                    <div className="text-xs text-sage-700">
                      Imported {formatTimestamp(activeSession.imported_at_ms)} ·{' '}
                      {activeSession.frames.length} frame(s)
                    </div>
                  </div>
                  <div className="text-xs font-medium text-sage-700">
                    {t(formatSpoilerMode(activeSession.spoiler_mode))}
                  </div>
                </div>
                {activeSession.analysis?.spoiler_note && (
                  <p className="mt-3 text-sm text-stone-700">
                    {activeSession.analysis.spoiler_note}
                  </p>
                )}
                {activeSession.analysis && (
                  <p className="mt-3 whitespace-pre-line text-sm text-stone-800">
                    {activeSession.analysis.recap}
                  </p>
                )}
              </div>

              <div className="grid gap-3 md:grid-cols-2">
                <div className="rounded-xl border border-stone-200 p-4">
                  <div className="text-sm font-semibold text-stone-900">Highlights</div>
                  <div className="mt-3 space-y-3">
                    {activeHighlights.length > 0 ? (
                      activeHighlights.map(highlight => (
                        <article
                          key={highlight.id}
                          className="rounded-lg border border-stone-200 bg-stone-50 p-3">
                          <div className="flex items-center justify-between gap-2">
                            <div className="font-medium text-stone-900">{highlight.title}</div>
                            <span className="rounded-full bg-white px-2 py-0.5 text-[11px] text-stone-500">
                              {(highlight.confidence * 100).toFixed(0)}%
                            </span>
                          </div>
                          <p className="mt-1 text-xs uppercase tracking-wide text-stone-500">
                            {highlight.kind} · frame {highlight.frame_index + 1} ·{' '}
                            {formatTimestamp(highlight.captured_at_ms)}
                          </p>
                          <p className="mt-2 text-sm text-stone-700">{highlight.rationale}</p>
                        </article>
                      ))
                    ) : (
                      <p className="text-sm text-stone-500">No highlights detected yet.</p>
                    )}
                  </div>
                </div>

                <div className="rounded-xl border border-stone-200 p-4">
                  <div className="text-sm font-semibold text-stone-900">Clip candidates</div>
                  <div className="mt-3 space-y-3">
                    {activeClips.length > 0 ? (
                      activeClips.map(clip => (
                        <article
                          key={clip.id}
                          className="rounded-lg border border-amber-100 bg-amber-50 p-3">
                          <div className="font-medium text-amber-950">{clip.start_label}</div>
                          <div className="text-xs text-amber-800">through {clip.end_label}</div>
                          <p className="mt-2 text-sm text-stone-700">{clip.rationale}</p>
                        </article>
                      ))
                    ) : (
                      <p className="text-sm text-stone-500">No clip candidates yet.</p>
                    )}
                  </div>
                </div>
              </div>

              <div className="rounded-xl border border-stone-200 p-4">
                <div className="text-sm font-semibold text-stone-900">Platform drafts</div>
                <div className="mt-3 grid gap-3 md:grid-cols-3">
                  {activeDrafts.length > 0 ? (
                    activeDrafts.map(draft => (
                      <article
                        key={`${draft.platform}-${draft.title}`}
                        className="rounded-lg border border-stone-200 bg-white p-3">
                        <div className="text-xs font-semibold uppercase tracking-wide text-stone-500">
                          {draft.platform}
                        </div>
                        <div className="mt-1 font-medium text-stone-900">{draft.title}</div>
                        <p className="mt-2 text-sm text-stone-700 whitespace-pre-line">
                          {draft.description}
                        </p>
                        <div className="mt-3 flex flex-wrap gap-2">
                          {draft.tags.map(tag => (
                            <span
                              key={tag}
                              className="rounded-full bg-stone-100 px-2 py-0.5 text-[11px] text-stone-600">
                              #{tag}
                            </span>
                          ))}
                        </div>
                      </article>
                    ))
                  ) : (
                    <p className="text-sm text-stone-500">
                      Draft metadata will appear after analysis.
                    </p>
                  )}
                </div>
              </div>

              <div className="rounded-xl border border-sage-200 bg-sage-50 p-4">
                <div className="text-sm font-semibold text-sage-900">Ask this session</div>
                <div className="mt-3 flex flex-col gap-3 lg:flex-row">
                  <input
                    value={question}
                    onChange={event => setQuestion(event.target.value)}
                    className="flex-1 rounded-xl border border-sage-200 bg-white px-3 py-2 outline-none ring-0 focus:border-primary-500"
                    placeholder="What were my best fights?"
                  />
                  <button
                    type="button"
                    onClick={handleQuestion}
                    disabled={questionBusy}
                    className="rounded-xl bg-sage-700 px-4 py-2 text-sm font-semibold text-white transition hover:bg-sage-800 disabled:cursor-not-allowed disabled:opacity-60">
                    {questionBusy ? 'Thinking…' : 'Ask session'}
                  </button>
                </div>
                {questionAnswer && (
                  <p className="mt-3 whitespace-pre-line text-sm text-stone-800">
                    {questionAnswer}
                  </p>
                )}
                {activeSession.analysis?.follow_up_questions &&
                  activeSession.analysis.follow_up_questions.length > 0 && (
                    <div className="mt-3 flex flex-wrap gap-2 text-xs text-stone-600">
                      {activeSession.analysis.follow_up_questions.map(item => (
                        <button
                          key={item}
                          type="button"
                          onClick={() => setQuestion(item)}
                          className="rounded-full border border-sage-200 bg-white px-3 py-1 transition hover:bg-sage-50">
                          {item}
                        </button>
                      ))}
                    </div>
                  )}
              </div>
            </div>
          ) : (
            <div className="rounded-xl border border-dashed border-stone-300 bg-stone-50 p-8 text-center text-sm text-stone-500">
              Import a folder of keyframes to create the first gameplay session.
            </div>
          )}
        </section>

        <aside className="space-y-4 rounded-2xl border border-stone-200 bg-white p-5 shadow-soft">
          <div>
            <h3 className="text-lg font-semibold text-stone-900">Session history</h3>
            <p className="text-sm text-stone-500">
              Previously imported or analyzed sessions for this workspace.
            </p>
          </div>
          <div className="space-y-3">
            {recentSessions.length > 0 ? (
              recentSessions.map(session => (
                <button
                  key={session.session_id}
                  type="button"
                  onClick={() => setActiveSession(session)}
                  className={`w-full rounded-xl border p-3 text-left transition ${
                    activeSession?.session_id === session.session_id
                      ? 'border-primary-300 bg-primary-50'
                      : 'border-stone-200 bg-white hover:bg-stone-50'
                  }`}>
                  <div className="flex items-start justify-between gap-3">
                    <div>
                      <div className="font-medium text-stone-900">{session.session_title}</div>
                      <div className="text-xs text-stone-500">{session.game_id}</div>
                    </div>
                    <div className="text-right text-[11px] text-stone-500">
                      <div>{t(formatSpoilerMode(session.spoiler_mode))}</div>
                      <div>{session.frames.length} frame(s)</div>
                    </div>
                  </div>
                  <div className="mt-2 text-xs text-stone-500">
                    Imported {formatTimestamp(session.imported_at_ms)}
                  </div>
                </button>
              ))
            ) : (
              <p className="text-sm text-stone-500">No saved sessions yet.</p>
            )}
          </div>
          <button
            type="button"
            onClick={() => void refreshSessions()}
            className="w-full rounded-xl border border-stone-200 bg-white px-4 py-2 text-sm font-semibold text-stone-700 transition hover:bg-stone-50">
            Refresh history
          </button>
        </aside>
      </div>
    </div>
  );
}
