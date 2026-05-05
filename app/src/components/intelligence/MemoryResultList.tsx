/**
 * Middle pane of MemoryWorkspace — time-grouped chunk rows.
 *
 * Sections: TODAY / YESTERDAY / THIS WEEK / OLDER, headers are sticky
 * so the user always knows which time bucket is on screen.
 *
 * Auto-scrolls to the active row on mount and on selection change.
 *
 * The list is intentionally non-virtualized for now — mock fixtures
 * top out at ~30 rows. Once real data lands we can swap in react-window
 * (or similar) without changing the public API.
 */
import { useEffect, useMemo, useRef } from 'react';

import type { Chunk } from '../../utils/tauriCommands';

interface MemoryResultListProps {
  chunks: Chunk[];
  selectedChunkId: string | null;
  onSelectChunk: (id: string) => void;
}

type GroupKey = 'TODAY' | 'YESTERDAY' | 'THIS WEEK' | 'OLDER';

interface Group {
  key: GroupKey;
  chunks: Chunk[];
}

const HOUR_MS = 60 * 60 * 1000;
const DAY_MS = 24 * HOUR_MS;

function startOfLocalDay(d: Date): Date {
  const out = new Date(d);
  out.setHours(0, 0, 0, 0);
  return out;
}

function bucketFor(
  ts: number,
  todayMs: number,
  yesterdayMs: number,
  weekStartMs: number
): GroupKey {
  if (ts >= todayMs) return 'TODAY';
  if (ts >= yesterdayMs) return 'YESTERDAY';
  if (ts >= weekStartMs) return 'THIS WEEK';
  return 'OLDER';
}

function pad2(n: number): string {
  return n < 10 ? `0${n}` : String(n);
}

const WEEKDAY_SHORT = ['Sun', 'Mon', 'Tue', 'Wed', 'Thu', 'Fri', 'Sat'];
const MONTH_SHORT = [
  'Jan',
  'Feb',
  'Mar',
  'Apr',
  'May',
  'Jun',
  'Jul',
  'Aug',
  'Sep',
  'Oct',
  'Nov',
  'Dec',
];

function formatTime(ts: number, group: GroupKey): string {
  const d = new Date(ts);
  if (group === 'TODAY' || group === 'YESTERDAY') {
    return `${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
  }
  if (group === 'THIS WEEK') {
    return `${WEEKDAY_SHORT[d.getDay()]} ${pad2(d.getHours())}:${pad2(d.getMinutes())}`;
  }
  return `${MONTH_SHORT[d.getMonth()]} ${d.getDate()}`;
}

function chunkSubject(chunk: Chunk): string {
  const preview = (chunk.content_preview ?? '').trim();
  if (!preview) return chunk.id;
  // Use the first sentence/line as the subject
  const firstLine = preview.split('\n')[0];
  const sentenceEnd = firstLine.search(/[.!?](?:\s|$)/);
  if (sentenceEnd > 16 && sentenceEnd < 120) {
    return firstLine.slice(0, sentenceEnd + 1).trim();
  }
  return firstLine.length > 140 ? `${firstLine.slice(0, 137)}…` : firstLine;
}

function chunkSenderLabel(chunk: Chunk): string {
  // Try to derive a sender from source_id; fall back to source kind.
  const left = chunk.source_id.split('|')[0];
  const after = left.includes(':') ? left.split(':').slice(1).join(':') : left;
  return after || chunk.source_kind;
}

export function MemoryResultList({
  chunks,
  selectedChunkId,
  onSelectChunk,
}: MemoryResultListProps) {
  const groups = useMemo<Group[]>(() => {
    const today = startOfLocalDay(new Date()).getTime();
    const yesterday = today - DAY_MS;
    const weekStart = today - 7 * DAY_MS;

    const buckets: Record<GroupKey, Chunk[]> = {
      TODAY: [],
      YESTERDAY: [],
      'THIS WEEK': [],
      OLDER: [],
    };
    for (const c of chunks) {
      buckets[bucketFor(c.timestamp_ms, today, yesterday, weekStart)].push(c);
    }
    const order: GroupKey[] = ['TODAY', 'YESTERDAY', 'THIS WEEK', 'OLDER'];
    return order.map(key => ({ key, chunks: buckets[key] })).filter(g => g.chunks.length > 0);
  }, [chunks]);

  const activeRowRef = useRef<HTMLButtonElement | null>(null);

  useEffect(() => {
    if (activeRowRef.current) {
      activeRowRef.current.scrollIntoView({ block: 'nearest' });
    }
  }, [selectedChunkId]);

  if (chunks.length === 0) {
    return (
      <section className="mw-pane-results" data-testid="memory-result-list">
        <div className="mw-results-empty">No matching chunks.</div>
      </section>
    );
  }

  return (
    <section className="mw-pane-results" data-testid="memory-result-list">
      <div className="mw-pane-scroll">
        {groups.map(group => (
          <div key={group.key} className="mw-results-section">
            <div className="mw-results-section-header">{group.key}</div>
            {group.chunks.map(chunk => {
              const isActive = chunk.id === selectedChunkId;
              return (
                <button
                  type="button"
                  key={chunk.id}
                  ref={isActive ? activeRowRef : undefined}
                  className={`mw-result-row${isActive ? ' is-active' : ''}`}
                  onClick={() => onSelectChunk(chunk.id)}
                  data-chunk-id={chunk.id}>
                  <span className="mw-result-time">
                    {formatTime(chunk.timestamp_ms, group.key)}
                  </span>
                  <span className="mw-result-content">
                    <span className="mw-result-subject">{chunkSubject(chunk)}</span>
                    <span className="mw-result-meta">
                      <span className="mw-result-kind">{chunk.source_kind}</span>
                      {' · '}
                      {chunkSenderLabel(chunk)} · {chunk.token_count} tok
                    </span>
                  </span>
                </button>
              );
            })}
          </div>
        ))}
      </div>
    </section>
  );
}
