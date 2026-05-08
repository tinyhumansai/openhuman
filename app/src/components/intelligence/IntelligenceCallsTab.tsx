import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import { useEffect, useState } from 'react';

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
        title: 'Joining call',
        message: 'Opening the Meet window — admit the agent from the host side.',
      });
    } catch (err) {
      const message = err instanceof Error ? err.message : 'Failed to start Meet call.';
      setError(message);
      onToast?.({ type: 'error', title: 'Could not start call', message });
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
      const message = err instanceof Error ? err.message : 'Failed to close call.';
      onToast?.({ type: 'error', title: 'Could not close call', message });
    }
  };

  return (
    <div className="space-y-6">
      <div>
        <h2 className="text-base font-semibold text-stone-900">Join a Google Meet call</h2>
        <p className="mt-1 text-sm text-stone-500">
          Paste a Meet link and the agent will join the call as a named guest in a separate window.
          The host needs to admit the agent from the Meet waiting room.
        </p>
      </div>

      <form onSubmit={handleSubmit} className="space-y-4">
        <label className="block">
          <span className="text-xs font-medium uppercase tracking-wide text-stone-500">
            Meet link
          </span>
          <input
            type="url"
            inputMode="url"
            autoComplete="off"
            spellCheck={false}
            value={meetUrl}
            onChange={e => setMeetUrl(e.target.value)}
            placeholder={PLACEHOLDER_URL}
            className="mt-1 w-full rounded-xl border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 placeholder:text-stone-400 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100"
            required
          />
        </label>

        <label className="block">
          <span className="text-xs font-medium uppercase tracking-wide text-stone-500">
            Display name
          </span>
          <input
            type="text"
            value={displayName}
            onChange={e => setDisplayName(e.target.value)}
            maxLength={64}
            className="mt-1 w-full rounded-xl border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 focus:border-primary-500 focus:outline-none focus:ring-2 focus:ring-primary-100"
            required
          />
        </label>

        {error && (
          <div
            role="alert"
            className="rounded-xl border border-coral-200 bg-coral-50 px-3 py-2 text-sm text-coral-700">
            {error}
          </div>
        )}

        <button
          type="submit"
          disabled={submitting || !meetUrl.trim() || !displayName.trim()}
          className="inline-flex items-center justify-center rounded-xl border border-primary-600 bg-primary-600 px-4 py-2 text-sm font-medium text-white shadow-soft transition hover:bg-primary-500 disabled:cursor-not-allowed disabled:opacity-50">
          {submitting ? 'Opening Meet…' : 'Join call'}
        </button>
      </form>

      {activeCalls.length > 0 && (
        <div className="space-y-2">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-stone-500">
            Active calls
          </h3>
          <ul className="space-y-2">
            {activeCalls.map(call => (
              <li
                key={call.requestId}
                className="flex items-center justify-between gap-3 rounded-xl border border-stone-200 bg-stone-50 px-3 py-2">
                <div className="min-w-0">
                  <div className="truncate text-sm font-medium text-stone-900">
                    {call.displayName}
                  </div>
                  <div className="truncate text-xs text-stone-500">{call.meetUrl}</div>
                </div>
                <button
                  type="button"
                  onClick={() => handleClose(call.requestId)}
                  className="shrink-0 rounded-lg border border-stone-200 bg-white px-3 py-1 text-xs text-stone-600 hover:border-coral-300 hover:text-coral-600">
                  Leave
                </button>
              </li>
            ))}
          </ul>
        </div>
      )}
    </div>
  );
}
