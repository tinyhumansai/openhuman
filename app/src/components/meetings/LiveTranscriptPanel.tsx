/**
 * LiveTranscriptPanel — renders meeting transcript turns as they stream in
 * during an active call (issue #4304).
 *
 * Fed by the `liveTranscript` buffer in `backendMeetSlice`, which accumulates
 * `agent_meetings:transcript_delta` events. The line at `partialIndex` is shown
 * greyed/italic until the backend finalizes it. The list autoscrolls to the
 * newest turn. On call end the buffer is reconciled away (the authoritative
 * final transcript takes over), so this panel naturally empties.
 */
import debug from 'debug';
import { useEffect, useRef } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { BackendMeetTurn } from '../../store/backendMeetSlice';

const log = debug('meetings:live-transcript');

/**
 * Live delta turns carry the speaker inline as a `[Name]` prefix (every human
 * speaker has `role: 'user'`, so the real identity lives in the content), and
 * may optionally be preceded by a `[MM:SS]` timestamp. Pull the leading
 * speaker tag out so it can be rendered as a label, mirroring how the final
 * transcript is shown via `parseTranscriptLine`. Falls back to no speaker when
 * the content has no tag (e.g. the assistant's own turns).
 */
const LIVE_SPEAKER_RE = /^\s*(?:\[\d{1,2}:\d{2}\]\s*)?\[([^\]]+)\]\s*([\s\S]*)$/;

function parseLiveLine(content: string): { speaker: string | null; text: string } {
  const match = LIVE_SPEAKER_RE.exec(content);
  if (match) return { speaker: match[1] ?? null, text: match[2] ?? '' };
  return { speaker: null, text: content };
}

export interface LiveTranscriptPanelProps {
  turns: BackendMeetTurn[];
  partialIndex: number | null;
}

export function LiveTranscriptPanel({ turns = [], partialIndex }: LiveTranscriptPanelProps) {
  const { t } = useT();
  const scrollRef = useRef<HTMLDivElement>(null);

  // `turns` is keyed by the backend transcript index and can be sparse (skipped
  // `[System]` turns leave gaps), so render only the populated slots while
  // keeping each turn's real index for the partial-line comparison. Default to
  // an empty array so a caller that hasn't seeded the live buffer (e.g. a store
  // built before this slice field existed) renders the empty state, not a crash.
  const rows = turns
    .map((turn, index) => ({ turn, index }))
    .filter((row): row is { turn: BackendMeetTurn; index: number } => Boolean(row.turn));

  // Autoscroll to the newest turn whenever a turn is added or the tail line is
  // updated (partial → final, or partial text extended).
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    el.scrollTop = el.scrollHeight;
    log('[live-transcript] autoscroll rows=%d partialIndex=%o', rows.length, partialIndex);
  }, [rows.length, partialIndex, turns]);

  return (
    <div className="mt-3 space-y-1">
      <div className="flex items-center gap-1.5">
        <span className="h-1.5 w-1.5 rounded-full bg-coral-500 animate-pulse" aria-hidden="true" />
        <p className="text-[10px] font-semibold uppercase tracking-wide text-content-muted">
          {t('skills.meetingBots.liveTranscriptHeading')}
        </p>
      </div>
      <div
        ref={scrollRef}
        className="max-h-40 overflow-y-auto rounded-md bg-surface-muted/70 p-2 space-y-0.5"
        aria-live="polite">
        {rows.length === 0 ? (
          <p className="text-[10px] italic text-content-faint">
            {t('skills.meetingBots.liveTranscriptEmpty')}
          </p>
        ) : (
          rows.map(({ turn, index }) => {
            const isAssistant = turn.role === 'assistant';
            const isPartial = index === partialIndex;
            const { speaker, text } = parseLiveLine(turn.content);
            return (
              <p
                key={index}
                className={[
                  'text-[10px]',
                  isAssistant ? 'text-primary-600 dark:text-primary-400' : 'text-content-secondary',
                  isPartial ? 'italic text-content-faint' : '',
                ]
                  .filter(Boolean)
                  .join(' ')}>
                {speaker && <span className="mr-1 font-medium">{speaker}:</span>}
                {text}
              </p>
            );
          })
        )}
      </div>
    </div>
  );
}

export default LiveTranscriptPanel;
