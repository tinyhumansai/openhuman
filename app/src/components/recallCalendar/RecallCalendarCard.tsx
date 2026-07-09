import { useT } from '../../lib/i18n/I18nContext';
import { useRecallCalendar } from '../../lib/recallCalendar/hooks';
import Button from '../ui/Button';
import { Spinner } from '../ui/icons';

/**
 * Card for connecting a Google Calendar via Recall.ai Calendar V1.
 *
 * Lives on the Meetings page (calendar connection is meeting-specific). Renders
 * only when the backend advertises the integration as enabled
 * (`RECALL_CALENDAR_ENABLED`). Connecting opens the Google OAuth consent flow in
 * the browser; once the status poll flips to connected, the hook switches Google
 * Meet detection to the Recall calendar source.
 */
function CalendarGlyph() {
  return (
    <svg
      width="20"
      height="20"
      viewBox="0 0 24 24"
      fill="none"
      aria-hidden="true"
      className="text-primary-500">
      <rect x="3" y="4.5" width="18" height="16" rx="2.5" stroke="currentColor" strokeWidth="1.6" />
      <path d="M3 9h18" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
      <path d="M8 2.5v4M16 2.5v4" stroke="currentColor" strokeWidth="1.6" strokeLinecap="round" />
    </svg>
  );
}

export default function RecallCalendarCard() {
  const { t } = useT();
  const { status, loading, busy, error, beginConnect, disconnect } = useRecallCalendar();

  // Hidden until the backend advertises the integration as enabled.
  if (loading || !status?.enabled) return null;

  const connected = status.connected;

  return (
    <div
      data-testid="recall-calendar-card"
      className="rounded-2xl border border-line bg-surface p-3 shadow-soft">
      <div className="flex items-center gap-3">
        {/* Leading icon tile */}
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-xl bg-primary-500/10">
          <CalendarGlyph />
        </div>

        {/* Title + status */}
        <div className="min-w-0 flex-1">
          <div className="text-sm font-semibold text-content-primary">
            {t('skills.recallCalendar.title')}
          </div>
          {connected ? (
            <div className="mt-0.5 flex items-center gap-1.5">
              <span
                className="h-1.5 w-1.5 shrink-0 rounded-full bg-emerald-500"
                aria-hidden="true"
              />
              <span className="truncate text-xs text-content-secondary">
                {status.email || t('skills.connected')}
              </span>
            </div>
          ) : (
            <div className="mt-0.5 truncate text-xs text-content-secondary">
              {t('skills.recallCalendar.description')}
            </div>
          )}
        </div>

        {/* Action */}
        {connected ? (
          <Button
            variant="tertiary"
            tone="danger"
            size="sm"
            disabled={busy}
            leadingIcon={busy ? <Spinner className="h-3.5 w-3.5" /> : undefined}
            onClick={() => void disconnect()}
            data-testid="recall-calendar-disconnect">
            {t('skills.disconnect')}
          </Button>
        ) : (
          <Button
            variant="primary"
            size="sm"
            disabled={busy}
            leadingIcon={busy ? <Spinner className="h-3.5 w-3.5" /> : undefined}
            onClick={() => void beginConnect()}
            data-testid="recall-calendar-connect">
            {t('skills.connect')}
          </Button>
        )}
      </div>

      {error && (
        <div className="mt-2 rounded-lg bg-red-500/10 px-2.5 py-1.5 text-[11px] text-red-500">
          {error}
        </div>
      )}
    </div>
  );
}
