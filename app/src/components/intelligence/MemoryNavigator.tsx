/**
 * Left pane of MemoryWorkspace — search box, slim heatmap header, and four
 * collapsible lens sections (recent / sources / people / topics).
 *
 * Selections are NOT mutually exclusive: multiple selected items intersect
 * the result list filter. Sections may be collapsed independently.
 */
import { useEffect, useMemo, useState } from 'react';

import type { Chunk, EntityRef, Source } from '../../utils/tauriCommands';
import { MemoryHeatmap } from './MemoryHeatmap';

export interface NavigatorSelection {
  sourceIds: string[];
  entityIds: string[];
}

interface MemoryNavigatorProps {
  chunks: Chunk[];
  sources: Source[];
  topPeople: EntityRef[];
  topTopics: EntityRef[];
  selection: NavigatorSelection;
  onSelectionChange: (next: NavigatorSelection) => void;
  searchQuery: string;
  onSearchChange: (next: string) => void;
}

const HOUR_MS = 60 * 60 * 1000;
const DAY_MS = 24 * HOUR_MS;

function dotClassFor(status: string | undefined): string {
  switch (status) {
    case 'admitted':
      return 'mw-dot dot-admitted';
    case 'pending_extraction':
      return 'mw-dot dot-pending';
    case 'buffered':
      return 'mw-dot dot-buffered';
    case 'dropped':
      return 'mw-dot dot-dropped';
    default:
      return 'mw-dot';
  }
}

interface SectionProps {
  label: string;
  defaultOpen?: boolean;
  countSummary?: string;
  children: React.ReactNode;
}

function NavSection({ label, defaultOpen = true, countSummary, children }: SectionProps) {
  const [open, setOpen] = useState(defaultOpen);
  return (
    <div className="mw-section">
      <button
        type="button"
        className="mw-section-heading"
        onClick={() => setOpen(o => !o)}
        aria-expanded={open}>
        <span>{label}</span>
        {countSummary && (
          <span
            style={{
              fontFamily: "'JetBrains Mono', monospace",
              fontSize: 10,
              letterSpacing: 0,
              color: 'var(--ink-whisper)',
              marginLeft: 8,
              textTransform: 'none',
            }}>
            {countSummary}
          </span>
        )}
        <span className={`mw-section-chev${open ? ' open' : ''}`} aria-hidden>
          ›
        </span>
      </button>
      {open && children}
    </div>
  );
}

export function MemoryNavigator({
  chunks,
  sources,
  topPeople,
  topTopics,
  selection,
  onSelectionChange,
  searchQuery,
  onSearchChange,
}: MemoryNavigatorProps) {
  const heatmapTimestamps = useMemo(
    () => chunks.map(c => Math.floor(c.timestamp_ms / 1000)),
    [chunks]
  );

  // Wall-clock-derived counts. Computed in an effect to keep render pure
  // (the `react-hooks/components-and-hooks-must-be-pure` rule rejects a
  // raw `Date.now()` call inside a `useMemo` body, since two equivalent
  // renders could produce different values).
  const [recentCounts, setRecentCounts] = useState<{ today: number; week: number }>({
    today: 0,
    week: 0,
  });
  useEffect(() => {
    const now = Date.now();
    const startOfDay = new Date(now);
    startOfDay.setHours(0, 0, 0, 0);
    const startOfDayMs = startOfDay.getTime();
    const startOfWeekMs = now - 7 * DAY_MS;
    let today = 0;
    let week = 0;
    for (const c of chunks) {
      if (c.timestamp_ms >= startOfDayMs) today++;
      if (c.timestamp_ms >= startOfWeekMs) week++;
    }
    setRecentCounts({ today, week });
  }, [chunks]);
  const { today: todayCount, week: weekCount } = recentCounts;

  const toggleSource = (id: string) => {
    const has = selection.sourceIds.includes(id);
    onSelectionChange({
      ...selection,
      sourceIds: has ? selection.sourceIds.filter(s => s !== id) : [...selection.sourceIds, id],
    });
  };

  const toggleEntity = (id: string) => {
    const has = selection.entityIds.includes(id);
    const next = has ? selection.entityIds.filter(s => s !== id) : [...selection.entityIds, id];
    console.debug(
      '[ui-flow][memory-navigator] toggleEntity id=%s wasActive=%o next=%o',
      id,
      has,
      next
    );
    onSelectionChange({ ...selection, entityIds: next });
  };

  const renderEntityList = (refs: EntityRef[]) => (
    <ul className="mw-list">
      {refs.map(ref => {
        const tag = `${ref.kind}/${ref.surface.replace(/\s+/g, '-')}`;
        // For tag-based selection we use the raw tag string; for entity_id we
        // compare against tags on chunks. Match either form to be lenient.
        const isActive =
          selection.entityIds.includes(tag) || selection.entityIds.includes(ref.entity_id);
        return (
          <li key={ref.entity_id}>
            <button
              type="button"
              className={`mw-list-item${isActive ? ' is-active' : ''}`}
              onClick={() => toggleEntity(ref.entity_id)}
              aria-pressed={isActive}>
              <span className="mw-dot" aria-hidden />
              <span className="mw-list-name" title={ref.surface}>
                {ref.surface}
              </span>
              <span className="mw-list-count">{ref.count}</span>
            </button>
          </li>
        );
      })}
      {refs.length === 0 && (
        <li style={{ padding: '6px 16px', fontSize: 12, color: 'var(--ink-whisper)' }}>—</li>
      )}
    </ul>
  );

  return (
    <aside className="mw-pane-navigator" data-testid="memory-navigator">
      <div className="mw-search-row">
        <input
          type="text"
          className="mw-search-input"
          placeholder="search memory…"
          value={searchQuery}
          onChange={e => onSearchChange(e.target.value)}
          aria-label="Search memory"
        />
      </div>

      <div className="mw-heatmap-host" data-testid="memory-navigator-heatmap">
        <MemoryHeatmap timestamps={heatmapTimestamps} />
      </div>

      <div className="mw-pane-scroll">
        <NavSection label="recent" defaultOpen>
          <div className="mw-recent-summary">
            <span>today {todayCount}</span>
            <span>this week {weekCount}</span>
          </div>
        </NavSection>

        <NavSection label="sources" defaultOpen countSummary={String(sources.length)}>
          {sources.length === 0 ? (
            <ul className="mw-list">
              <li style={{ padding: '6px 16px', fontSize: 12, color: 'var(--ink-whisper)' }}>—</li>
            </ul>
          ) : (
            (() => {
              // Group sources by their source_kind ('email', 'slack', 'chat', …)
              // and render each kind as its own nested collapsible. Lets the
              // user filter at the kind level (drill into Email vs Slack) and
              // then by individual sender within each.
              const byKind = new Map<string, Source[]>();
              for (const s of sources) {
                const arr = byKind.get(s.source_kind) ?? [];
                arr.push(s);
                byKind.set(s.source_kind, arr);
              }
              const kindLabel: Record<string, string> = {
                email: 'Email',
                slack: 'Slack',
                chat: 'Chat',
                document: 'Documents',
              };
              const kinds = Array.from(byKind.entries()).sort((a, b) => b[1].length - a[1].length);
              return (
                <div>
                  {kinds.map(([kind, kindSources]) => (
                    <NavSection
                      key={kind}
                      label={kindLabel[kind] ?? kind}
                      defaultOpen={false}
                      countSummary={String(kindSources.length)}>
                      <ul className="mw-list">
                        {kindSources.map(src => {
                          const isActive = selection.sourceIds.includes(src.source_id);
                          return (
                            <li key={src.source_id}>
                              <button
                                type="button"
                                className={`mw-list-item${isActive ? ' is-active' : ''}`}
                                onClick={() => toggleSource(src.source_id)}
                                aria-pressed={isActive}>
                                <span className={dotClassFor(src.lifecycle_status)} aria-hidden />
                                <span className="mw-list-name" title={src.display_name}>
                                  {src.display_name}
                                </span>
                                <span className="mw-list-count">{src.chunk_count}</span>
                              </button>
                            </li>
                          );
                        })}
                      </ul>
                    </NavSection>
                  ))}
                </div>
              );
            })()
          )}
        </NavSection>

        <NavSection label="people" defaultOpen countSummary={String(topPeople.length)}>
          {renderEntityList(topPeople)}
        </NavSection>

        <NavSection label="topics" defaultOpen countSummary={String(topTopics.length)}>
          {renderEntityList(topTopics)}
        </NavSection>
      </div>
    </aside>
  );
}
