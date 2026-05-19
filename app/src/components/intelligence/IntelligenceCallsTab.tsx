import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { closeMeetCall, joinMeetCall } from '../../services/meetCallService';

type ActiveCall = { requestId: string; meetUrl: string; displayName: string };

type Props = {
  onToast?: (toast: {
    type: 'success' | 'error' | 'info';
    title: string;
    message?: string;
  }) => void;
};

const PLACEHOLDER_URL = 'https://meet.google.com/abc-defg-hij';

/**
 * Calls tab on the Intelligence page.
 *
 * Lets the user paste a Google Meet link, choose a display name, and have
 * the agent join the call as an anonymous guest in a dedicated CEF
 * webview window. The window itself is opened by the Tauri shell — this
 * component just collects inputs, fires the RPC + invoke pair, and
 * tracks active calls so the user can close them from the same surface.
 */
export default function IntelligenceCallsTab({ onToast }: Props) {
  const { t } = useT();
  const [meetUrl, setMeetUrl] = useState('');
  const [displayName, setDisplayName] = useState('OpenHuman Agent');
  const [submitting, setSubmitting] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [activeCalls, setActiveCalls] = useState<ActiveCall[]>([]);

  // Listen for shell-emitted close events so the in-flight list stays
  // accurate when the user closes a Meet window directly. Outside the
  // Tauri shell `listen` rejects with a transport error — we swallow it.
  useEffect(() => {
    let unlisten: UnlistenFn | undefined;
    let cancelled = false;

    listen<{ request_id: string }>('meet-call:closed', event => {
      const closedId = event.payload?.request_id;
      if (!closedId) return;
      setActiveCalls(prev => prev.filter(call => call.requestId !== closedId));
    })
      .then(stop => {
        if (cancelled) stop();
        else unlisten = stop;
      })
      .catch(() => {
        // Browser dev surface — no Tauri event bridge available.
      });

    return () => {
      cancelled = true;
      if (unlisten) unlisten();
    };
  }, []);

  const handleSubmit = async (event: React.FormEvent<HTMLFormElement>) => {
    event.preventDefault();
    setError(null);
    setSubmitting(true);
    try {
      const result = await joinMeetCall({ meetUrl, displayName });
      setActiveCalls(prev => [
        ...prev.filter(call => call.requestId !== result.requestId),
        { requestId: result.requestId, meetUrl: result.meetUrl, displayName: result.displayName },
      ]);
      setMeetUrl('');
      onToast?.({
        type: 'success',
        title: t('calls.joiningCall'),
        message: t('calls.meetWindowOpening'),
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : t('calls.failedToStart');
      setError(message);
      onToast?.({ type: 'error', title: t('calls.couldNotStart'), message });
    } finally {
      setSubmitting(false);
    }
  };

  const handleClose = async (requestId: string) => {
    try {
      const closed = await closeMeetCall(requestId);
      if (closed) {
        // Only drop the row when the shell confirms the window is gone.
        // The `meet-call:closed` event listener also clears the row, so
        // a manual window-close still keeps the list accurate.
        setActiveCalls(prev => prev.filter(call => call.requestId !== requestId));
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : t('calls.failedToClose');
      onToast?.({ type: 'error', title: t('calls.couldNotClose'), message });
    }
  };

  // Suppress unused-variable warnings while the UI is hidden behind Coming Soon.
  void t;
  void meetUrl;
  void setMeetUrl;
  void displayName;
  void setDisplayName;
  void submitting;
  void error;
  void activeCalls;
  void handleSubmit;
  void handleClose;
  void PLACEHOLDER_URL;

  return (
    <div className="flex flex-col items-center justify-center py-16 px-6 text-center">
      <div className="mb-4 flex h-14 w-14 items-center justify-center rounded-2xl bg-primary-50 dark:bg-primary-500/10">
        <svg
          className="h-7 w-7 text-primary-500"
          fill="none"
          viewBox="0 0 24 24"
          stroke="currentColor"
          strokeWidth={1.5}>
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            d="M2.25 6.75c0 8.284 6.716 15 15 15h2.25a2.25 2.25 0 0 0 2.25-2.25v-1.372c0-.516-.351-.966-.852-1.091l-4.423-1.106c-.44-.11-.902.055-1.173.417l-.97 1.293c-.282.376-.769.542-1.21.38a12.035 12.035 0 0 1-7.143-7.143c-.162-.441.004-.928.38-1.21l1.293-.97c.363-.271.527-.734.417-1.173L6.963 3.102a1.125 1.125 0 0 0-1.091-.852H4.5A2.25 2.25 0 0 0 2.25 4.5v2.25Z"
          />
        </svg>
      </div>
      <h2 className="text-base font-semibold text-stone-900 dark:text-neutral-100">Calls</h2>
      <p className="mt-2 text-sm text-stone-500 dark:text-neutral-400 max-w-xs">
        AI-assisted calls are coming soon. Stay tuned.
      </p>
      <span className="mt-4 inline-flex items-center rounded-full bg-primary-50 dark:bg-primary-500/10 px-3 py-1 text-xs font-medium text-primary-600 dark:text-primary-400">
        Coming Soon
      </span>
    </div>
  );

  /* Original Calls UI — re-enable when the feature is ready
  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-base font-semibold text-stone-900 dark:text-neutral-100">
          {t('calls.joinMeet')}
        </h2>
        <p className="mt-1 text-sm text-stone-500 dark:text-neutral-400">
          {t('calls.joinMeetDescription')}
        </p>
      </div>

      <form onSubmit={handleSubmit} className="space-y-4">
        <label className="block">
          <span className="text-xs font-medium uppercase tracking-wide text-stone-500 dark:text-neutral-400">
            {t('calls.meetLink')}
          </span>
          <input
            type="url"
            inputMode="url"
            autoComplete="off"
            spellCheck={false}
            value={meetUrl}
            onChange={e => setMeetUrl(e.target.value)}
            placeholder={PLACEHOLDER_URL}
            className="mt-1 w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder:text-stone-400 dark:text-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100"
            required
          />
        </label>

        <label className="block">
          <span className="text-xs font-medium uppercase tracking-wide text-stone-500 dark:text-neutral-400">
            {t('calls.displayName')}
          </span>
          <input
            type="text"
            value={displayName}
            onChange={e => setDisplayName(e.target.value)}
            maxLength={64}
            className="mt-1 w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100"
            required
          />
        </label>

        {error && (
          <div
            role="alert"
            className="rounded-xl border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-3 py-2 text-sm text-coral-700 dark:text-coral-300">
            {error}
          </div>
        )}

        <button
          type="submit"
          disabled={submitting || !meetUrl.trim() || !displayName.trim()}
          className="inline-flex items-center justify-center rounded-xl border border-primary-600 bg-primary-600 px-4 py-2 text-sm font-medium text-white shadow-soft transition hover:bg-primary-500 disabled:cursor-not-allowed disabled:opacity-50">
          {submitting ? t('calls.openingMeet') : t('calls.joinCall')}
        </button>
      </form>

      {activeCalls.length > 0 && (
        <div className="space-y-2">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
            {t('calls.activeCalls')}
          </h3>
          <ul className="space-y-2">
            {activeCalls.map(call => (
              <li
                key={call.requestId}
                className="flex items-center justify-between gap-3 rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2">
                <div className="min-w-0">
                  <div className="truncate text-sm font-medium text-stone-900 dark:text-neutral-100">
                    {call.displayName}
                  </div>
                  <div className="truncate text-xs text-stone-500 dark:text-neutral-400">
                    {call.meetUrl}
                  </div>
                </div>
                <button
                  type="button"
                  onClick={() => handleClose(call.requestId)}
                  className="shrink-0 rounded-lg border border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 px-3 py-1 text-xs text-stone-600 dark:text-neutral-300 hover:border-coral-300 hover:text-coral-600 dark:text-coral-300">
                  {t('calls.leave')}
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
  */
}
