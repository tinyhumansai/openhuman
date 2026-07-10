import { useCallback, useEffect, useState } from 'react';
import { LuKeyRound, LuX } from 'react-icons/lu';

import { useT } from '../../../../lib/i18n/I18nContext';
import {
  type ClaudeCodeAuthStatus,
  openhumanClaudeCodeAuthStatus,
  openhumanClaudeCodeLoginLaunch,
  openhumanClaudeCodeSetFullAccess,
  openhumanClaudeCodeSettings,
} from '../../../../utils/tauriCommands/config';
import Button from '../../../ui/Button';

/**
 * Claude Code CLI connect control — the peer of the Codex connect button.
 *
 * Inline: a "Claude Code" button + a one-line status summary. Clicking the
 * button opens a modal with the actual controls (enable/disable, sign-in /
 * reconnect, install hint).
 *
 * Auth is probed via `claude auth status --json` (cross-platform: covers the
 * macOS Keychain as well as the Linux/Windows file stores) or
 * `ANTHROPIC_API_KEY`. We do NOT spawn the slow `claude --version` probe — a
 * missing/old binary surfaces as `unknown` from the auth probe, rendered as a
 * compact install hint rather than "signed out".
 */
export function ClaudeCodeConnect({
  connected,
  busy = false,
  onConnect,
  onDisconnect,
}: {
  connected: boolean;
  busy?: boolean;
  onConnect: () => void | Promise<void>;
  onDisconnect: () => void | Promise<void>;
}) {
  const { t } = useT();
  const [open, setOpen] = useState(false);
  const [auth, setAuth] = useState<ClaudeCodeAuthStatus | null>(null);
  const [authLoading, setAuthLoading] = useState(false);
  const [acting, setActing] = useState(false);

  const probeAuth = useCallback(async () => {
    setAuthLoading(true);
    try {
      // Resolves to the BARE AuthStatus (no `{ result }` envelope) — see the
      // wrapper in tauriCommands/config.ts.
      const resp = await openhumanClaudeCodeAuthStatus();
      setAuth(resp);
    } catch {
      setAuth(null);
    } finally {
      setAuthLoading(false);
    }
  }, []);

  // Probe once connected so the inline summary + modal reflect sign-in state.
  // The disconnected render is DERIVED from `connected` (`shownAuth` below)
  // rather than clearing `auth` from the effect — synchronous setState in an
  // effect body is disallowed by `react-hooks/set-state-in-effect`.
  useEffect(() => {
    if (connected) void probeAuth();
  }, [connected, probeAuth]);

  const shownAuth = connected ? auth : null;

  const runConnect = async () => {
    setActing(true);
    try {
      await onConnect();
    } finally {
      setActing(false);
    }
  };
  const runDisconnect = async () => {
    setActing(true);
    try {
      await onDisconnect();
    } finally {
      setActing(false);
    }
  };

  return (
    <div className="flex flex-wrap items-center gap-2">
      <Button
        variant="secondary"
        size="sm"
        onClick={() => setOpen(true)}
        leadingIcon={<LuKeyRound className="h-3.5 w-3.5" />}>
        {t('settings.ai.claudeCode.button')}
      </Button>
      <span className="text-xs text-content-muted">
        <InlineSummary connected={connected} auth={shownAuth} loading={authLoading} />
      </span>

      {open && (
        <ClaudeCodeModal
          connected={connected}
          busy={busy || acting}
          auth={shownAuth}
          authLoading={authLoading}
          onClose={() => setOpen(false)}
          onConnect={runConnect}
          onDisconnect={runDisconnect}
          onRecheck={probeAuth}
        />
      )}
    </div>
  );
}

/** Title-case a raw subscription type (`"max"` → `"Max"`) for display. */
function formatSubscriptionType(raw: string): string {
  const trimmed = raw.trim();
  if (!trimmed) return trimmed;
  return trimmed.charAt(0).toUpperCase() + trimmed.slice(1);
}

/** Heuristic: does an `unknown` reason indicate the binary is missing? */
function looksNotInstalled(reason: string | null): boolean {
  if (!reason) return false;
  const r = reason.toLowerCase();
  return r.includes('not found') || r.includes('not installed') || r.includes('path');
}

/** One-line status shown next to the inline "Claude Code" button. */
function InlineSummary({
  connected,
  auth,
  loading,
}: {
  connected: boolean;
  auth: ClaudeCodeAuthStatus | null;
  loading: boolean;
}) {
  const { t } = useT();
  if (!connected) {
    return <>{t('settings.ai.claudeCode.inlineNotConnected')}</>;
  }
  if (!auth) {
    return (
      <>
        {loading
          ? t('settings.ai.claudeCode.checkingSignIn')
          : t('settings.ai.claudeCode.inlineConnected')}
      </>
    );
  }
  if (auth.source === 'subscription') {
    const who = auth.account_email ?? t('settings.ai.claudeCode.subscriptionFallback');
    const plan = auth.subscription_type
      ? ` (${formatSubscriptionType(auth.subscription_type)})`
      : '';
    return (
      <span className="text-emerald-600 dark:text-emerald-400">
        {t('settings.ai.claudeCode.signedInAs')} {who}
        {plan}
      </span>
    );
  }
  if (auth.source === 'api_key_env') {
    return (
      <span className="text-emerald-600 dark:text-emerald-400">
        {t('settings.ai.claudeCode.usingApiKeyEnv')}
      </span>
    );
  }
  if (auth.source === 'unknown') {
    return (
      <span className="text-amber-600 dark:text-amber-400">
        {looksNotInstalled(auth.reason)
          ? t('settings.ai.claudeCode.cliNotInstalled')
          : t('settings.ai.claudeCode.signInUnknown')}
      </span>
    );
  }
  return (
    <span className="text-amber-600 dark:text-amber-400">
      {t('settings.ai.claudeCode.connectedNotSignedIn')}
    </span>
  );
}

/**
 * Modal with the actual Claude Code controls: enable/disable the provider,
 * sign in / reconnect via the CLI, and install guidance.
 */
function ClaudeCodeModal({
  connected,
  busy,
  auth,
  authLoading,
  onClose,
  onConnect,
  onDisconnect,
  onRecheck,
}: {
  connected: boolean;
  busy: boolean;
  auth: ClaudeCodeAuthStatus | null;
  authLoading: boolean;
  onClose: () => void;
  onConnect: () => void | Promise<void>;
  onDisconnect: () => void | Promise<void>;
  onRecheck: () => void | Promise<void>;
}) {
  const { t } = useT();
  const [launching, setLaunching] = useState(false);
  const [launchError, setLaunchError] = useState<string | null>(null);

  // Persisted full-access toggle (bypassPermissions vs the default acceptEdits).
  // `null` until loaded so the switch can render a disabled placeholder.
  const [fullAccess, setFullAccess] = useState<boolean | null>(null);
  const [savingAccess, setSavingAccess] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const s = await openhumanClaudeCodeSettings();
        if (!cancelled) setFullAccess(s.full_access);
      } catch {
        // Fail safe to OFF (acceptEdits) if the read fails.
        if (!cancelled) setFullAccess(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const toggleFullAccess = async (next: boolean) => {
    setSavingAccess(true);
    setFullAccess(next); // optimistic
    try {
      const s = await openhumanClaudeCodeSetFullAccess(next);
      setFullAccess(s.full_access);
    } catch {
      setFullAccess(!next); // revert on failure
    } finally {
      setSavingAccess(false);
    }
  };

  const launchLogin = async () => {
    setLaunching(true);
    setLaunchError(null);
    try {
      await openhumanClaudeCodeLoginLaunch();
    } catch {
      // Surface the failure inline rather than leaving an unhandled rejection.
      setLaunchError(t('settings.ai.claudeCode.loginError'));
    } finally {
      setLaunching(false);
    }
  };

  return (
    <div
      role="dialog"
      aria-modal="true"
      aria-label={t('settings.ai.claudeCode.modalTitle')}
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/30 p-4"
      onClick={onClose}>
      <div
        className="w-full max-w-md rounded-2xl border border-line bg-surface p-6 shadow-soft"
        onClick={e => e.stopPropagation()}>
        <div className="mb-4 flex items-start justify-between gap-3">
          <div>
            <h3 className="text-base font-semibold text-content">
              {t('settings.ai.claudeCode.modalTitle')}
            </h3>
            <p className="mt-1 max-w-sm text-xs leading-5 text-content-muted">
              {t('settings.ai.claudeCode.modalDescription')}
            </p>
          </div>
          <Button
            iconOnly
            variant="tertiary"
            size="xs"
            onClick={onClose}
            aria-label={t('settings.ai.claudeCode.close')}>
            <LuX className="h-4 w-4" />
          </Button>
        </div>

        {/* Connection */}
        <div className="flex items-center justify-between gap-3 rounded-lg border border-line px-3 py-2">
          <div className="text-xs">
            <div className="font-medium text-content">{t('settings.ai.claudeCode.connection')}</div>
            <div
              className={
                connected ? 'text-emerald-600 dark:text-emerald-400' : 'text-content-muted'
              }>
              {connected
                ? t('settings.ai.claudeCode.enabled')
                : t('settings.ai.claudeCode.notEnabled')}
            </div>
          </div>
          {connected ? (
            <Button
              variant="secondary"
              tone="danger"
              size="sm"
              onClick={() => void onDisconnect()}
              disabled={busy}>
              {busy
                ? t('settings.ai.claudeCode.disconnecting')
                : t('settings.ai.claudeCode.disconnect')}
            </Button>
          ) : (
            <button
              type="button"
              onClick={() => void onConnect()}
              disabled={busy}
              className="rounded-md bg-neutral-900 px-2.5 py-1 text-xs font-medium text-white hover:bg-neutral-700 disabled:opacity-50 dark:bg-neutral-100 dark:text-neutral-900 dark:hover:bg-neutral-300">
              {busy ? t('settings.ai.claudeCode.enabling') : t('settings.ai.claudeCode.enable')}
            </button>
          )}
        </div>

        {/* Authentication */}
        <div className="mt-3 rounded-lg border border-line px-3 py-2">
          <div className="mb-1.5 flex items-center justify-between">
            <span className="text-xs font-medium text-content">
              {t('settings.ai.claudeCode.authentication')}
            </span>
            <Button
              variant="tertiary"
              size="xs"
              onClick={() => void onRecheck()}
              disabled={authLoading}>
              {authLoading
                ? t('settings.ai.claudeCode.checking')
                : t('settings.ai.claudeCode.recheck')}
            </Button>
          </div>
          <AuthDetail auth={auth} loading={authLoading} />
          <div className="mt-2">
            <Button
              variant="secondary"
              size="sm"
              onClick={() => void launchLogin()}
              disabled={launching}>
              {launching
                ? t('settings.ai.claudeCode.openingTerminal')
                : auth?.source === 'none'
                  ? t('settings.ai.claudeCode.signIn')
                  : t('settings.ai.claudeCode.reconnect')}
            </Button>
            <p className="mt-1.5 text-[11px] text-content-muted">
              {t('settings.ai.claudeCode.loginHint')}
            </p>
            {launchError && (
              <p className="mt-1 text-[11px] text-rose-600 dark:text-rose-400" role="alert">
                {launchError}
              </p>
            )}
          </div>
        </div>

        {/* Permissions — full access vs. the default acceptEdits posture. */}
        <div className="mt-3 rounded-lg border border-line px-3 py-2">
          <div className="flex items-center justify-between gap-3">
            <div className="min-w-0">
              <div className="text-xs font-medium text-content">
                {t('settings.ai.claudeCode.fullAccess')}
              </div>
              <p className="mt-0.5 text-[11px] leading-4 text-content-muted">
                {fullAccess
                  ? t('settings.ai.claudeCode.fullAccessOn')
                  : t('settings.ai.claudeCode.fullAccessOff')}
              </p>
            </div>
            <button
              type="button"
              role="switch"
              aria-checked={fullAccess === true}
              aria-label={t('settings.ai.claudeCode.fullAccess')}
              disabled={fullAccess === null || savingAccess}
              onClick={() => void toggleFullAccess(!fullAccess)}
              className={`relative inline-flex h-5 w-9 shrink-0 items-center rounded-full transition-colors disabled:cursor-wait disabled:opacity-50 ${
                fullAccess ? 'bg-emerald-500 dark:bg-emerald-500' : 'bg-surface-strong'
              }`}>
              <span
                className={`inline-block h-4 w-4 transform rounded-full bg-surface shadow transition-transform ${
                  fullAccess ? 'translate-x-4' : 'translate-x-0.5'
                }`}
              />
            </button>
          </div>
          <p className="mt-1.5 text-[11px] leading-4 text-content-faint">
            {isMac()
              ? t('settings.ai.claudeCode.sandboxNoteMac')
              : t('settings.ai.claudeCode.sandboxNoteOther')}
          </p>
        </div>
      </div>
    </div>
  );
}

/** Best-effort macOS detection for the permissions copy (UA-based). */
function isMac(): boolean {
  if (typeof navigator === 'undefined') return false;
  return /Mac|iPhone|iPad/i.test(navigator.platform || navigator.userAgent || '');
}

/** Detailed auth line inside the modal. */
function AuthDetail({ auth, loading }: { auth: ClaudeCodeAuthStatus | null; loading: boolean }) {
  const { t } = useT();
  if (!auth) {
    return (
      <p className="text-xs text-content-muted">
        {loading
          ? t('settings.ai.claudeCode.checkingSignIn')
          : t('settings.ai.claudeCode.enableToCheck')}
      </p>
    );
  }
  if (auth.source === 'subscription') {
    const who = auth.account_email ?? t('settings.ai.claudeCode.subscriptionFallback');
    const plan = auth.subscription_type
      ? ` (${formatSubscriptionType(auth.subscription_type)})`
      : '';
    return (
      <p className="text-xs text-emerald-600 dark:text-emerald-400">
        {t('settings.ai.claudeCode.signedInAs')} {who}
        {plan}
      </p>
    );
  }
  if (auth.source === 'api_key_env') {
    return (
      <p className="text-xs text-emerald-600 dark:text-emerald-400">
        {t('settings.ai.claudeCode.usingApiKeyEnvDetail')}
      </p>
    );
  }
  if (auth.source === 'unknown') {
    if (looksNotInstalled(auth.reason)) {
      return (
        <p className="text-xs text-amber-600 dark:text-amber-400">
          {t('settings.ai.claudeCode.notFoundInstall')}
        </p>
      );
    }
    return (
      <p className="text-xs text-amber-600 dark:text-amber-400">
        {t('settings.ai.claudeCode.unknownDetail')}
      </p>
    );
  }
  return (
    <p className="text-xs text-amber-600 dark:text-amber-400">
      {t('settings.ai.claudeCode.notSignedIn')}
    </p>
  );
}
