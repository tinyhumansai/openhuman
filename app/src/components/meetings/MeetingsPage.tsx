/**
 * Meetings page orchestrator.
 *
 * Renders the Beta banner, the active-meeting overlay (when a bot is running),
 * the meeting composer (when idle), and the recent-calls history below.
 *
 * Owns the `hasSubmittedRef` success-toast pattern — the ref lives here so the
 * toast fires reliably even though the inline composer unmounts when status
 * flips to 'active' (same pattern as the original `MeetingBotsCard`).
 */
import debug from 'debug';
import { useEffect, useRef, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { selectBackendMeetStatus } from '../../store/backendMeetSlice';
import { useAppSelector } from '../../store/hooks';
import { isTauri, openhumanGetMeetSettings } from '../../utils/tauriCommands';
import BetaBanner from '../ui/BetaBanner';
import { ActiveMeetingBanner } from './ActiveMeetingBanner';
import HistorySection from './HistorySection';
import { MeetComposer } from './MeetComposer';
import { MeetDefaultsDrawer } from './MeetDefaultsDrawer';
import { UpcomingTable } from './UpcomingTable';

const log = debug('meetings:page');

type Toast = { type: 'success' | 'error' | 'info'; title: string; message?: string };

export interface MeetingsPageProps {
  onToast?: (toast: Toast) => void;
}

export default function MeetingsPage({ onToast }: MeetingsPageProps) {
  const { t } = useT();
  const status = useAppSelector(selectBackendMeetStatus);
  const [drawerOpen, setDrawerOpen] = useState(false);
  // watchCalendar: null = unknown (don't show hint), false = off (show hint when there are meetings)
  const [watchCalendar, setWatchCalendar] = useState<boolean | null>(null);
  // Show the live banner while joining or in an active meeting. All other
  // states ('idle', 'ended', 'error') render the composer so the user can
  // submit a new join or see the inline error from a failed attempt.
  const showActive = status === 'joining' || status === 'active';

  // `hasSubmittedRef` lives in this always-mounted parent so the success toast
  // fires reliably. When a join succeeds, `status` flips to 'active' and this
  // component swaps `MeetComposer` → `ActiveMeetingBanner`, unmounting the
  // composer before any effect inside it could observe 'active'. The composer
  // sets this ref on submit; we fire the success toast here.
  const hasSubmittedRef = useRef(false);
  useEffect(() => {
    if (!hasSubmittedRef.current) return;
    if (status === 'active') {
      hasSubmittedRef.current = false;
      log('[page] join succeeded → status=active, firing success toast');
      onToast?.({
        type: 'success',
        title: t('skills.meetingBots.joiningTitle'),
        message: t('skills.meetingBots.joiningMessage'),
      });
    }
  }, [status, onToast, t]);

  // Fetch watch_calendar once on mount (stale-60s is fine; re-fetched when drawer closes).
  useEffect(() => {
    if (!isTauri()) return;
    let cancelled = false;
    openhumanGetMeetSettings()
      .then(resp => {
        if (!cancelled) {
          log('[page] watch_calendar=%s', resp.result.watch_calendar);
          setWatchCalendar(resp.result.watch_calendar ?? false);
        }
      })
      .catch(err => {
        log('[page] failed to fetch meet settings for watchCalendar: %o', err);
        // Leave null → no hint shown (fail open, don't nag)
      });
    return () => {
      cancelled = true;
    };
  }, []);

  return (
    <div className="space-y-3 animate-fade-up">
      {/* Page header row: beta badge + gear button */}
      <div className="flex items-center justify-between">
        <BetaBanner />
        <button
          type="button"
          aria-label={t('skills.meetingBots.defaults.openDefaults')}
          onClick={() => setDrawerOpen(true)}
          className="p-1.5 rounded text-content-secondary hover:text-content-primary hover:bg-surface-hover transition-colors">
          <svg width="16" height="16" viewBox="0 0 24 24" fill="none" aria-hidden="true">
            <path
              d="M12 15a3 3 0 1 0 0-6 3 3 0 0 0 0 6Z"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
            <path
              d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1-2.83 2.83l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83-2.83l.06-.06A1.65 1.65 0 0 0 4.68 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 2.83-2.83l.06.06A1.65 1.65 0 0 0 9 4.68a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 2.83l-.06.06A1.65 1.65 0 0 0 19.4 9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1Z"
              stroke="currentColor"
              strokeWidth="1.5"
              strokeLinecap="round"
              strokeLinejoin="round"
            />
          </svg>
        </button>
      </div>

      {showActive ? (
        <ActiveMeetingBanner onToast={onToast} />
      ) : (
        <MeetComposer onToast={onToast} hasSubmittedRef={hasSubmittedRef} />
      )}

      <UpcomingTable watchCalendar={watchCalendar} />

      <HistorySection />

      <MeetDefaultsDrawer
        open={drawerOpen}
        onClose={() => {
          setDrawerOpen(false);
          // Re-fetch watch_calendar after drawer closes so the hint updates.
          if (!isTauri()) return;
          openhumanGetMeetSettings()
            .then(resp => setWatchCalendar(resp.result.watch_calendar ?? false))
            .catch(() => {
              /* leave unchanged */
            });
        }}
      />
    </div>
  );
}
