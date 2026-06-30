/**
 * HistorySection — orchestrates the two-column call-history view.
 *
 * Left column: HistoryRail (search, filter, date groups).
 * Right column: HistoryDetail (detail for the selected call).
 *
 * Fetches listMeetCalls(50) on mount with two delayed retries to catch
 * asynchronous writes from the core (same pattern as old MeetingsPage).
 */
import debug from 'debug';
import { useCallback, useEffect, useMemo, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { listMeetCalls, type MeetCallRecord } from '../../services/meetCallService';
import HistoryDetail from './HistoryDetail';
import HistoryRail, { type CallGroup } from './HistoryRail';
import { inferPlatformFromUrl } from './meetingUtils';

const log = debug('meetings:history');

/** UTC day key for grouping: "YYYY-MM-DD". */
function utcDayKey(ms: number): string {
  const d = new Date(ms);
  return `${d.getUTCFullYear()}-${String(d.getUTCMonth() + 1).padStart(2, '0')}-${String(d.getUTCDate()).padStart(2, '0')}`;
}

function todayKey(): string {
  return utcDayKey(Date.now());
}

function yesterdayKey(): string {
  return utcDayKey(Date.now() - 86400000);
}

function groupRecords(
  records: MeetCallRecord[],
  todayLabel: string,
  yesterdayLabel: string,
  earlierLabel: string
): CallGroup[] {
  const today = todayKey();
  const yesterday = yesterdayKey();

  const todayCalls: MeetCallRecord[] = [];
  const yesterdayCalls: MeetCallRecord[] = [];
  const earlierCalls: MeetCallRecord[] = [];

  for (const r of records) {
    const key = utcDayKey(r.started_at_ms);
    if (key === today) todayCalls.push(r);
    else if (key === yesterday) yesterdayCalls.push(r);
    else earlierCalls.push(r);
  }

  const groups: CallGroup[] = [];
  if (todayCalls.length > 0) groups.push({ label: todayLabel, calls: todayCalls });
  if (yesterdayCalls.length > 0) groups.push({ label: yesterdayLabel, calls: yesterdayCalls });
  if (earlierCalls.length > 0) groups.push({ label: earlierLabel, calls: earlierCalls });
  return groups;
}

export function HistorySection() {
  const { t } = useT();
  const [records, setRecords] = useState<MeetCallRecord[] | null>(null);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  // The ID explicitly chosen by the user. May be null (no explicit pick yet)
  // or point to a call that's been filtered out — effectiveCallId handles both.
  const [selectedCallId, setSelectedCallId] = useState<string | null>(null);
  const [searchQuery, setSearchQuery] = useState('');
  const [platformFilter, setPlatformFilter] = useState('');

  const fetchCalls = useCallback(async () => {
    log('[history] fetching calls');
    try {
      const rows = await listMeetCalls(50);
      log('[history] loaded %d calls', rows.length);
      // Clear any previous error only after a successful fetch so the UI
      // doesn't flicker between error and loading on retry.
      setError(null);
      setRecords(rows);
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to load calls.';
      log('[history] fetch error', err);
      console.warn('[meetings:history] listMeetCalls failed:', err);
      setError(message);
      setRecords([]);
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    // Wrap the initial call in setTimeout so the rule's transitive analysis
    // does not flag setState calls (which are all async-after-await in fetchCalls)
    // as synchronous within the effect body.
    const id = setTimeout(() => void fetchCalls(), 0);
    const retries = [1200, 3000].map(delay => setTimeout(() => void fetchCalls(), delay));
    return () => {
      clearTimeout(id);
      retries.forEach(clearTimeout);
    };
  }, [fetchCalls]);

  // Apply search + platform filter
  const filteredRecords = useMemo(() => {
    if (!records) return [];
    return records.filter(r => {
      // Platform filter
      if (platformFilter) {
        const inferred = inferPlatformFromUrl(r.meet_url);
        if (inferred !== platformFilter) return false;
      }
      // Search query
      if (searchQuery.trim()) {
        const q = searchQuery.toLowerCase();
        const code = (() => {
          try {
            return new URL(r.meet_url).pathname.replace(/^\/+/, '');
          } catch {
            return r.meet_url;
          }
        })();
        const participantStr = (r.participants ?? []).join(' ').toLowerCase();
        const owner = (r.owner_display_name ?? '').toLowerCase();
        if (!code.toLowerCase().includes(q) && !participantStr.includes(q) && !owner.includes(q)) {
          return false;
        }
      }
      return true;
    });
  }, [records, searchQuery, platformFilter]);

  const groups = useMemo(
    () =>
      groupRecords(
        filteredRecords,
        t('skills.meetingBots.history.today'),
        t('skills.meetingBots.history.yesterday'),
        t('skills.meetingBots.history.earlier')
      ),
    [filteredRecords, t]
  );

  // Derive the effective selection during render — no setState in an effect:
  // • null  when no records survive the active filter (clears a stale selection)
  // • first visible call when nothing is explicitly selected or the selected
  //   call was filtered out (auto-snap keeps the detail pane populated)
  // • the user's explicit pick when it is still visible in filteredRecords
  const effectiveCallId = useMemo<string | null>(() => {
    if (filteredRecords.length === 0) return null;
    if (selectedCallId !== null && filteredRecords.some(r => r.request_id === selectedCallId)) {
      return selectedCallId;
    }
    return filteredRecords[0].request_id;
  }, [filteredRecords, selectedCallId]);

  const selectedRecord = useMemo(
    () => records?.find(r => r.request_id === effectiveCallId) ?? null,
    [records, effectiveCallId]
  );

  function handleSelect(id: string) {
    log('[history] selected call', id);
    setSelectedCallId(id);
  }

  return (
    <div className="space-y-3 rounded-2xl border border-line bg-surface p-4 shadow-soft">
      <div className="flex items-baseline justify-between">
        <h3 className="text-[11px] font-semibold uppercase tracking-wide text-content-muted">
          {t('skills.meetingBots.recentCallsHeading')}
          {records && records.length > 0 && (
            <span className="ml-1 text-content-faint normal-case font-normal">
              ({records.length})
            </span>
          )}
        </h3>
      </div>

      {error && <p className="text-[11px] text-coral-600 dark:text-coral-400">{error}</p>}

      {loading && records === null ? (
        <p className="text-[11px] text-content-faint">
          {t('skills.meetingBots.recentCallsLoading')}
        </p>
      ) : (
        <div className="grid grid-cols-1 gap-4 md:grid-cols-[280px_1fr]">
          {/* Left: Rail — on narrow screens hide when a call is selected */}
          <div className={effectiveCallId ? 'hidden md:block' : undefined}>
            <HistoryRail
              groups={groups}
              selectedId={effectiveCallId}
              onSelect={handleSelect}
              searchQuery={searchQuery}
              onSearchChange={setSearchQuery}
              platformFilter={platformFilter}
              onPlatformChange={setPlatformFilter}
            />
          </div>

          {/* Right: Detail — on narrow screens show only when something is selected */}
          <div className={!effectiveCallId ? 'hidden md:block' : undefined}>
            <HistoryDetail record={selectedRecord} />
          </div>
        </div>
      )}
    </div>
  );
}

export default HistorySection;
