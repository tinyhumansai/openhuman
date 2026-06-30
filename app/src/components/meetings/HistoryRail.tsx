/**
 * HistoryRail — the left-hand call list with search + platform filter.
 *
 * Renders date-grouped rows; each row is a button showing the platform logo,
 * meeting code, relative time, and turn count. The selected row is highlighted.
 */
import debug from 'debug';
import { useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import type { MeetCallRecord, MeetingPlatform } from '../../services/meetCallService';
import {
  inferPlatformFromUrl,
  MEETING_PLATFORMS,
  platformLabel,
  platformLogoUrl,
} from './meetingUtils';

const log = debug('meetings:rail');

function ChevronDownIcon() {
  return (
    <svg
      width="12"
      height="12"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2.5"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="m6 9 6 6 6-6" />
    </svg>
  );
}

function AllPlatformsIcon() {
  // Funnel / filter glyph for the "All platforms" (no filter) state.
  return (
    <svg
      width="16"
      height="16"
      viewBox="0 0 24 24"
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true">
      <path d="M22 3H2l8 9.46V19l4 2v-8.54L22 3z" />
    </svg>
  );
}

/**
 * Compact platform filter: a button showing only the selected platform's icon
 * (or a funnel glyph for "all"), opening a menu that lists each platform with
 * its icon AND name.
 */
function PlatformFilterMenu({ value, onChange }: { value: string; onChange: (p: string) => void }) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  const ref = useRef<HTMLDivElement>(null);

  useEffect(() => {
    if (!open) return;
    function onDocPointer(e: MouseEvent) {
      if (ref.current && !ref.current.contains(e.target as Node)) setOpen(false);
    }
    document.addEventListener('mousedown', onDocPointer);
    return () => document.removeEventListener('mousedown', onDocPointer);
  }, [open]);

  const selected = value ? (value as MeetingPlatform) : null;
  const allLabel = t('skills.meetingBots.history.allPlatforms');

  function pick(p: string) {
    onChange(p);
    setOpen(false);
  }

  return (
    <div ref={ref} className="relative shrink-0">
      <button
        type="button"
        onClick={() => setOpen(o => !o)}
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-label={selected ? platformLabel(selected, t) : allLabel}
        title={selected ? platformLabel(selected, t) : allLabel}
        className="flex items-center gap-1 rounded-lg border border-line bg-surface px-2 py-1.5 text-content-secondary hover:border-primary-300 focus:outline-none focus:ring-1 focus:ring-primary-400">
        {selected ? (
          <img
            src={platformLogoUrl(selected)}
            alt=""
            width={16}
            height={16}
            className="h-4 w-4 shrink-0 rounded-sm object-contain"
          />
        ) : (
          <AllPlatformsIcon />
        )}
        <ChevronDownIcon />
      </button>

      {open && (
        <ul
          role="listbox"
          className="absolute left-0 z-20 mt-1 min-w-[170px] overflow-hidden rounded-lg border border-line bg-surface py-1 shadow-soft">
          <li>
            <button
              type="button"
              role="option"
              aria-selected={!value}
              onClick={() => pick('')}
              className={[
                'flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px]',
                !value
                  ? 'text-primary-700 dark:text-primary-300'
                  : 'text-content-secondary hover:bg-surface-muted dark:hover:bg-surface-muted/40',
              ].join(' ')}>
              <AllPlatformsIcon />
              <span className="flex-1">{allLabel}</span>
            </button>
          </li>
          {MEETING_PLATFORMS.map(p => {
            const isSel = value === p;
            return (
              <li key={p}>
                <button
                  type="button"
                  role="option"
                  aria-selected={isSel}
                  onClick={() => pick(p)}
                  className={[
                    'flex w-full items-center gap-2 px-3 py-1.5 text-left text-[12px]',
                    isSel
                      ? 'text-primary-700 dark:text-primary-300'
                      : 'text-content-secondary hover:bg-surface-muted dark:hover:bg-surface-muted/40',
                  ].join(' ')}>
                  <img
                    src={platformLogoUrl(p)}
                    alt=""
                    width={16}
                    height={16}
                    className="h-4 w-4 shrink-0 rounded-sm object-contain"
                  />
                  <span className="flex-1">{platformLabel(p, t)}</span>
                </button>
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
}

export interface CallGroup {
  label: string;
  calls: MeetCallRecord[];
}

interface HistoryRailProps {
  groups: CallGroup[];
  selectedId: string | null;
  onSelect: (id: string) => void;
  searchQuery: string;
  onSearchChange: (q: string) => void;
  platformFilter: string;
  onPlatformChange: (p: string) => void;
}

function extractMeetingCode(url: string): string {
  try {
    return new URL(url).pathname.replace(/^\/+/, '') || url;
  } catch {
    return url;
  }
}

/**
 * Format a past timestamp as a compact relative label ("1h ago", "yesterday").
 *
 * All user-visible strings are routed through i18n. The caller must
 * supply the `t` function from `useT()`.
 */
function formatRelativeTime(ms: number, t: (key: string) => string): string {
  if (!ms) return '—';
  const diff = Date.now() - ms;
  if (diff < 0) return t('skills.meetingBots.relative.now');
  const seconds = Math.floor(diff / 1000);
  if (seconds < 60) return t('skills.meetingBots.relative.now');
  const minutes = Math.floor(seconds / 60);
  if (minutes < 60) {
    return t('skills.meetingBots.relative.minutesAgo').replace('{count}', String(minutes));
  }
  const hours = Math.floor(minutes / 60);
  if (hours < 24) {
    return t('skills.meetingBots.relative.hoursAgo').replace('{count}', String(hours));
  }
  const days = Math.floor(hours / 24);
  if (days === 1) return t('skills.meetingBots.relative.yesterday');
  if (days < 7) {
    return t('skills.meetingBots.relative.daysAgo').replace('{count}', String(days));
  }
  try {
    return new Date(ms).toLocaleDateString(undefined, { month: 'short', day: 'numeric' });
  } catch {
    return '—';
  }
}

export function HistoryRail({
  groups,
  selectedId,
  onSelect,
  searchQuery,
  onSearchChange,
  platformFilter,
  onPlatformChange,
}: HistoryRailProps) {
  const { t } = useT();

  const totalCalls = groups.reduce((sum, g) => sum + g.calls.length, 0);

  return (
    <div className="flex flex-col gap-2 min-h-0">
      {/* Compact platform filter (icon-only) + search */}
      <div className="flex items-center gap-2">
        <PlatformFilterMenu value={platformFilter} onChange={onPlatformChange} />
        <input
          type="search"
          value={searchQuery}
          onChange={e => onSearchChange(e.target.value)}
          placeholder={t('skills.meetingBots.history.searchPlaceholder')}
          className="min-w-0 flex-1 rounded-lg border border-line bg-surface px-2.5 py-1.5 text-[12px] text-content placeholder:text-content-faint focus:outline-none focus:ring-1 focus:ring-primary-400"
        />
      </div>

      {/* Groups */}
      <div className="flex-1 overflow-y-auto space-y-3 min-h-0">
        {totalCalls === 0 && (
          <p className="text-[11px] text-content-faint px-1">
            {t('skills.meetingBots.recentCallsEmpty')}
          </p>
        )}
        {groups.map(group => (
          <div key={group.label}>
            <p className="px-1 pb-0.5 text-[10px] font-semibold uppercase tracking-wide text-content-muted">
              {group.label}
            </p>
            <ul className="space-y-0.5">
              {group.calls.map(call => {
                const isSelected = call.request_id === selectedId;
                const code = extractMeetingCode(call.meet_url);
                const platform = inferPlatformFromUrl(call.meet_url);

                return (
                  <li key={call.request_id}>
                    <button
                      type="button"
                      onClick={() => {
                        log('[rail] selected call', call.request_id);
                        onSelect(call.request_id);
                      }}
                      className={[
                        'w-full rounded-lg px-2 py-1.5 text-left text-[11px] transition-colors',
                        isSelected
                          ? 'bg-primary-50 dark:bg-primary-900/20 text-primary-700 dark:text-primary-300'
                          : 'hover:bg-surface-muted dark:hover:bg-surface-muted/40 text-content-secondary',
                      ].join(' ')}>
                      <div className="flex items-center gap-1.5">
                        {platform && (
                          <img
                            src={platformLogoUrl(platform)}
                            alt={platformLabel(platform, t)}
                            width={16}
                            height={16}
                            className="h-4 w-4 shrink-0 rounded-sm object-contain"
                          />
                        )}
                        <span className="flex-1 truncate font-mono text-[11px]">{code}</span>
                        <span className="shrink-0 text-[10px] text-content-faint">
                          {formatRelativeTime(call.started_at_ms, t)}
                        </span>
                      </div>
                      <div className="mt-0.5 pl-5 text-[10px] text-content-muted">
                        {t(
                          call.turn_count === 1
                            ? 'skills.meetingBots.recentCallTurnSingular'
                            : 'skills.meetingBots.recentCallTurnPlural'
                        ).replace('{count}', String(call.turn_count))}
                      </div>
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}
      </div>
    </div>
  );
}

export default HistoryRail;
