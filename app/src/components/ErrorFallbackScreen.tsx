import { useState } from 'react';

import { useT } from '../lib/i18n/I18nContext';
import { isAnalyticsEnabled } from '../services/analytics';
import { LATEST_APP_DOWNLOAD_URL, SUPPORT_URL } from '../utils/config';
import { openUrl } from '../utils/openUrl';
import { safeInvoke as invoke } from '../utils/tauriCommands/common';

/**
 * ErrorFallbackScreen
 *
 * Full-screen recovery UI shown when the Sentry ErrorBoundary catches
 * a catastrophic React render error. Self-contained with zero dependencies
 * on Redux, Router, or any context provider.
 *
 * Errors caught by the boundary are auto-forwarded to Sentry by the
 * `Sentry.ErrorBoundary` wrapper in `App.tsx` (subject to user analytics
 * consent enforced in `analytics.ts::beforeSend`). When the capture
 * produces an event id, it is surfaced here as a copyable Error ID so the
 * user can share it with support, and a support deep link is pre-seeded
 * with it. When the user hasn't opted into analytics (no event id), the
 * Error ID / support affordances are hidden rather than shown empty.
 *
 * Recovery escalates: `Try recover` (in-place `resetError`) → `Reload app`
 * (hard reload to /home) → `Reveal logs` (open the logs folder so a user
 * trapped by a deterministic crash can still pull diagnostics for support).
 */

interface ErrorFallbackScreenProps {
  error: unknown;
  componentStack?: string;
  /** Sentry event id for the captured crash, when analytics is enabled. */
  eventId?: string | null;
  onReset: () => void;
}

export default function ErrorFallbackScreen({
  error,
  componentStack,
  eventId,
  onReset,
}: ErrorFallbackScreenProps) {
  const { t } = useT();
  const [copied, setCopied] = useState(false);
  const errorName = error instanceof Error ? error.name : 'Error';
  const errorMessage = error instanceof Error ? error.message : String(error);
  // Only surface the Error ID / support ref when analytics is on: with consent
  // off, `analytics.ts::beforeSend` drops the event, yet the SDK still hands the
  // boundary a generated id — showing it would let an opted-out user copy a ref
  // support can never look up (Codex P2 on #3980).
  const hasEventId = typeof eventId === 'string' && eventId.length > 0 && isAnalyticsEnabled();

  const copyEventId = async () => {
    if (!hasEventId) return;
    try {
      await navigator.clipboard.writeText(eventId as string);
      setCopied(true);
      setTimeout(() => setCopied(false), 1800);
    } catch {
      // Clipboard unavailable (permissions / non-secure context) — no-op;
      // the id stays visible for manual copy.
    }
  };

  const revealLogs = () => {
    // Diagnostics escape hatch: works even when the UI is otherwise dead.
    // `.catch` swallows the rejection on non-desktop / pre-bootstrap so a
    // fire-and-forget invoke can't surface as an unhandled rejection.
    void invoke('reveal_logs_folder').catch(() => {});
  };

  const openSupport = () => {
    if (!hasEventId) return;
    // `&` when SUPPORT_URL already carries a query (env override) so the ref
    // never produces a malformed double-`?` link.
    const sep = SUPPORT_URL.includes('?') ? '&' : '?';
    openUrl(`${SUPPORT_URL}${sep}ref=${encodeURIComponent(eventId as string)}`);
  };

  return (
    <div className="fixed inset-0 flex items-center justify-center bg-gradient-to-b from-stone-950 to-stone-900">
      <div className="w-full max-w-lg mx-4 bg-stone-900 border border-coral-500/30 rounded-2xl shadow-large overflow-hidden">
        {/* Accent bar */}
        <div className="h-1 bg-coral-500" />

        <div className="p-8">
          {/* Icon */}
          <div className="flex justify-center mb-6">
            <div className="w-16 h-16 rounded-full bg-coral-500/10 flex items-center justify-center">
              <svg
                className="w-8 h-8 text-coral-500"
                viewBox="0 0 24 24"
                fill="none"
                stroke="currentColor"
                strokeWidth={1.5}>
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  d="M12 9v3.75m-9.303 3.376c-.866 1.5.217 3.374 1.948 3.374h14.71c1.73 0 2.813-1.874 1.948-3.374L13.949 3.378c-.866-1.5-3.032-1.5-3.898 0L2.697 16.126zM12 15.75h.007v.008H12v-.008z"
                />
              </svg>
            </div>
          </div>

          {/* Title */}
          <h1 className="text-xl font-semibold text-white text-center mb-2">
            {t('app.errorFallback.heading')}
          </h1>
          <p className="text-sm text-content-faint text-center mb-6">
            {t('app.errorFallback.subheading')}
          </p>
          <p className="text-xs text-content-muted text-center mb-6">
            {t('app.errorFallback.hint')}
          </p>

          {/* Sentry Event ID — copyable; hidden when analytics produced no id */}
          {hasEventId && (
            <div className="flex items-center justify-between gap-3 bg-stone-800/50 border border-stone-700/50 rounded-xl px-3 py-2.5 mb-4">
              <div className="flex flex-col min-w-0">
                <span className="text-[10px] uppercase tracking-wide text-content-muted">
                  {t('app.errorFallback.eventIdLabel')}
                </span>
                <span className="font-mono text-xs text-content-faint truncate">{eventId}</span>
              </div>
              <button
                onClick={copyEventId}
                className={`flex-none text-xs font-medium rounded-lg px-3 py-1.5 transition-colors ${
                  copied
                    ? 'bg-primary-500/20 text-primary-300'
                    : 'bg-stone-700 hover:bg-stone-600 text-white'
                }`}>
                {copied ? t('app.errorFallback.eventIdCopied') : t('app.errorFallback.copyEventId')}
              </button>
            </div>
          )}

          {/* Error details */}
          <div className="bg-stone-800/50 border border-stone-700/50 rounded-xl p-4 mb-6">
            <p className="text-sm font-medium text-coral-400 mb-1">{errorName}</p>
            <p className="text-xs text-content-faint break-words">{errorMessage}</p>
            {componentStack && (
              <details className="mt-3">
                <summary className="text-xs text-content-muted cursor-pointer hover:text-content-faint transition-colors">
                  {t('app.errorFallback.componentStack')}
                </summary>
                <pre className="mt-2 text-[11px] text-content-muted whitespace-pre-wrap break-words max-h-[200px] overflow-auto">
                  {componentStack}
                </pre>
              </details>
            )}
          </div>

          {/* Primary actions — escalate recover → reload → reveal logs */}
          <div className="grid grid-cols-1 sm:grid-cols-3 gap-3">
            <button
              onClick={onReset}
              className="bg-stone-700 hover:bg-stone-600 text-white text-sm font-medium rounded-xl px-4 py-3 transition-colors">
              {t('app.errorFallback.tryRecover')}
            </button>
            <button
              onClick={() => {
                window.location.hash = '#/home';
                window.location.reload();
              }}
              className="bg-coral-500 hover:bg-coral-600 text-content-inverted text-sm font-medium rounded-xl px-4 py-3 transition-colors">
              {t('app.errorFallback.reloadApp')}
            </button>
            {/* Always rendered — `isTauri()` is false during the CEF IPC
                bootstrap gap, which is exactly when an early deterministic
                crash needs this escape hatch; the invoke fails safe off-desktop
                (Codex P2 on #3980). */}
            <button
              onClick={revealLogs}
              className="bg-stone-800 hover:bg-stone-700 text-white text-sm font-medium rounded-xl px-4 py-3 transition-colors border border-stone-600">
              {t('app.errorFallback.revealLogs')}
            </button>
          </div>

          {/* Secondary links */}
          <div className="flex items-center justify-center gap-4 mt-5 text-xs">
            {hasEventId && (
              <button
                onClick={openSupport}
                className="text-primary-400 hover:text-primary-300 hover:underline transition-colors">
                {t('app.errorFallback.contactSupport')}
              </button>
            )}
            <button
              onClick={() => openUrl(LATEST_APP_DOWNLOAD_URL)}
              className="text-content-muted hover:text-content-faint hover:underline transition-colors">
              {t('app.errorFallback.downloadLatest')}
            </button>
          </div>
        </div>
      </div>
    </div>
  );
}
