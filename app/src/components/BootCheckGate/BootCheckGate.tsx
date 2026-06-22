/**
 * BootCheckGate — pre-router gate rendered before the rest of the app mounts.
 *
 * Responsibilities:
 *   1. First-ever launch: prompt user to pick Local or Cloud core mode.
 *   2. Subsequent launches: run version / reachability check and block until
 *      the result is `match`.
 *
 * Visual language matches the onboarding hero design: centered card, soft
 * shadow, font-display headings, sage/primary accents, card-based choices.
 */
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { type BootCheckResult, runBootCheck } from '../../lib/bootCheck';
import { useT } from '../../lib/i18n/I18nContext';
import { bootCheckTransport, recoverPortConflict } from '../../services/bootCheckService';
import {
  clearCoreRpcTokenCache,
  clearCoreRpcUrlCache,
  testCoreRpcConnection,
} from '../../services/coreRpcClient';
import { type CoreMode, resetCoreMode, setCoreMode } from '../../store/coreModeSlice';
import { useAppDispatch, useAppSelector } from '../../store/hooks';
import {
  clearStoredCoreMode,
  clearStoredCoreToken,
  isLocalOrPrivateNetworkHost,
  normalizeRpcUrl,
  storeCoreMode,
  storeCoreToken,
  storeRpcUrl,
} from '../../utils/configPersistence';
import { isTauri } from '../../utils/tauriCommands/common';
import AppBackground from '../AppBackground';
import LanguageSelect from '../LanguageSelect';

const log = debug('boot-check');
const logError = debug('boot-check:error');

/**
 * Plain HTTP to a public host is insecure (unencrypted traffic), but we no
 * longer block it — return a non-blocking warning string so the UI can nudge
 * the user toward HTTPS while still letting them proceed. Returns null when the
 * URL is empty, unparseable, HTTPS, or points at a local/private host.
 */
function httpPublicHostWarning(
  rawUrl: string,
  t: (key: string, fallback?: string) => string
): string | null {
  const trimmed = rawUrl.trim();
  if (!trimmed) return null;
  try {
    const parsed = new URL(normalizeRpcUrl(trimmed));
    if (parsed.protocol === 'http:' && !isLocalOrPrivateNetworkHost(parsed.hostname)) {
      return t('bootCheck.httpPublicWarning');
    }
  } catch {
    // Unparseable URL — the error path in validateInputs handles messaging.
  }
  return null;
}

// ---------------------------------------------------------------------------
// Internal types
// ---------------------------------------------------------------------------

type Phase =
  | 'picker' // mode not set — show mode selector
  | 'checking' // boot check in flight
  | 'result'; // check finished with a non-match result

// ---------------------------------------------------------------------------
// Shared layout: Onboarding Hero Panel
// ---------------------------------------------------------------------------

interface PanelProps {
  children: React.ReactNode;
}

function Panel({ children }: PanelProps) {
  return (
    <div
      className="fixed inset-0 z-[10000] flex items-center justify-center overflow-y-auto p-4"
      style={{ backgroundColor: 'var(--color-background)' }}>
      <AppBackground />
      <div className="relative z-10 my-auto max-h-[calc(100vh-2rem)] w-full max-w-2xl overflow-y-auto rounded-2xl bg-white dark:bg-neutral-900 p-6 shadow-soft animate-fade-up sm:p-10">
        {children}
      </div>
    </div>
  );
}

function BootCheckLanguageSelect() {
  const { t } = useT();
  return (
    <div className="absolute right-6 top-6">
      <LanguageSelect id="boot-check-language" ariaLabel={t('settings.language')} />
    </div>
  );
}

// ---------------------------------------------------------------------------
// Picker (first-ever launch)
// ---------------------------------------------------------------------------

interface PickerProps {
  onConfirm: (mode: CoreMode) => void;
}

type TestStatus =
  | { kind: 'idle' }
  | { kind: 'testing' }
  | { kind: 'ok' }
  | { kind: 'auth' }
  | { kind: 'unreachable'; reason: string };

// Desktop release artifact URL surfaced on the web build's mode picker so
// users without a remote core have a clear path to install the app instead
// of being trapped on the cloud-only form.
const DESKTOP_DOWNLOAD_URL = 'https://github.com/tinyhumansai/openhuman/releases/latest';

type Accent = 'sage' | 'primary';

const ACCENT_CLASSES: Record<Accent, { selected: string; dot: string }> = {
  sage: {
    selected: '!border-sage-300 dark:!border-sage-600 bg-sage-50 dark:bg-sage-500/10 shadow-sm',
    dot: 'bg-sage-400',
  },
  primary: {
    selected:
      '!border-primary-400 dark:!border-primary-600 bg-primary-50 dark:bg-primary-500/15 shadow-sm',
    dot: 'bg-primary-400',
  },
};

interface ModeCardProps {
  selected: boolean;
  accent: Accent;
  onClick: () => void;
  title: string;
  description: string;
  testId: string;
}

function ModeCard({ selected, accent, onClick, title, description, testId }: ModeCardProps) {
  const accentClasses = ACCENT_CLASSES[accent];
  return (
    <button
      type="button"
      onClick={onClick}
      aria-pressed={selected}
      data-testid={testId}
      className={`flex flex-col rounded-2xl border-2 p-5 text-left transition-colors focus:outline-none h-full ${
        selected
          ? accentClasses.selected
          : '!border-stone-200 dark:!border-neutral-700 bg-white dark:bg-neutral-900 hover:!border-stone-300 dark:hover:!border-neutral-600 hover:bg-stone-50 dark:hover:bg-neutral-800/60'
      }`}>
      <h3 className="text-base font-semibold text-stone-900 dark:text-neutral-100">{title}</h3>
      <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400">{description}</p>
    </button>
  );
}

function ModePicker({ onConfirm }: PickerProps) {
  const { t } = useT();
  // Web build cannot spawn a local sidecar, so the only viable choice is
  // cloud. Default the selection accordingly and hide the local option in
  // the render path below.
  const isDesktop = isTauri();
  const [selected, setSelected] = useState<'local' | 'cloud'>(isDesktop ? 'local' : 'cloud');
  const [cloudUrl, setCloudUrl] = useState('');
  const [cloudToken, setCloudToken] = useState('');
  const [urlError, setUrlError] = useState<string | null>(null);
  const [urlWarning, setUrlWarning] = useState<string | null>(null);
  const [tokenError, setTokenError] = useState<string | null>(null);
  const [testStatus, setTestStatus] = useState<TestStatus>({ kind: 'idle' });

  /**
   * Validate the cloud URL + token inputs against a live core before we
   * commit the mode. We hit the public `core.ping` (auth-bypass) to confirm
   * reachability, then re-issue the same JSON-RPC envelope with the bearer
   * token to confirm `/rpc` accepts it. This catches the two most common
   * paste-time mistakes — wrong URL, wrong/missing token — with one click,
   * before the user lands on the unreachable result screen.
   *
   * Tokens are never logged: only `tokenLen` is emitted via the existing
   * picker debug line, and any error messages from the network/JSON parse
   * paths are passed through verbatim without the bearer value.
   */
  const validateInputs = (): { url: string; token: string } | null => {
    const rawUrl = cloudUrl.trim();
    if (!rawUrl) {
      setUrlError(t('bootCheck.invalidUrl'));
      return null;
    }
    const normalizedUrl = normalizeRpcUrl(rawUrl);
    try {
      const parsed = new URL(normalizedUrl);
      if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
        setUrlError(t('bootCheck.urlMustStartWith'));
        return null;
      }
    } catch {
      setUrlError(t('bootCheck.validUrlRequired'));
      return null;
    }
    setUrlError(null);

    const trimmedToken = cloudToken.trim();
    if (!trimmedToken) {
      setTokenError(t('bootCheck.tokenRequired'));
      return null;
    }
    setTokenError(null);

    return { url: normalizedUrl, token: trimmedToken };
  };

  const handleTestConnection = async () => {
    const validated = validateInputs();
    if (!validated) return;

    setTestStatus({ kind: 'testing' });
    log(
      '[boot-check] picker — testing cloud connection url=%s tokenLen=%d',
      validated.url,
      validated.token.length
    );

    try {
      const response = await testCoreRpcConnection(validated.url, validated.token);
      if (response.status === 401 || response.status === 403) {
        log('[boot-check] picker — test failed: auth (status=%d)', response.status);
        setTestStatus({ kind: 'auth' });
        return;
      }
      if (!response.ok) {
        log('[boot-check] picker — test failed: HTTP %d', response.status);
        setTestStatus({ kind: 'unreachable', reason: `HTTP ${response.status} from /rpc` });
        return;
      }
      // Drain the body — response.ok with JSON-RPC error is still reachable.
      try {
        await response.json();
      } catch {
        // Non-JSON body is unusual but doesn't disprove reachability.
      }
      log('[boot-check] picker — test succeeded');
      setTestStatus({ kind: 'ok' });
    } catch (err) {
      const reason = err instanceof Error ? err.message : 'Connection failed';
      logError('[boot-check] picker — test errored: %o', err);
      setTestStatus({ kind: 'unreachable', reason });
    }
  };

  const handleContinue = () => {
    if (selected === 'local') {
      log('[boot-check] picker — user selected local mode');
      onConfirm({ kind: 'local' });
      return;
    }

    const validated = validateInputs();
    if (!validated) return;

    log(
      '[boot-check] picker — user selected cloud mode url=%s tokenLen=%d',
      validated.url,
      validated.token.length
    );
    onConfirm({ kind: 'cloud', url: validated.url, token: validated.token });
  };

  return (
    <Panel>
      <BootCheckLanguageSelect />

      {/* Hero header — centered, onboarding style */}
      <div className="flex flex-col items-center text-center">
        <img src="/logo.png" alt="OpenHuman" className="w-16 h-16 rounded-xl mb-5" />
        <h1 className="text-2xl font-display text-stone-900 dark:text-neutral-100 mb-2 leading-tight">
          {t('bootCheck.heroTitle')}
        </h1>
        <p className="text-stone-500 dark:text-neutral-400 text-sm leading-relaxed max-w-md">
          {isDesktop ? t('bootCheck.heroDesktopDesc') : t('bootCheck.heroWebDesc')}
        </p>
      </div>

      {!isDesktop && (
        <div
          className="mt-6 rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-4 text-xs text-stone-600 dark:text-neutral-300 text-center"
          data-testid="web-download-cta">
          {t('bootCheck.preferDesktop')}{' '}
          <a
            href={DESKTOP_DOWNLOAD_URL}
            target="_blank"
            rel="noopener noreferrer"
            className="text-primary-500 underline hover:text-primary-600">
            {t('bootCheck.downloadDesktop')}
          </a>
          .
        </div>
      )}

      {/* Mode selection cards — two-column grid like onboarding */}
      <div className="mt-6 grid grid-cols-1 gap-3 sm:grid-cols-2 sm:items-stretch">
        {/* Local option — desktop only */}
        {isDesktop && (
          <ModeCard
            testId="boot-check-mode-local"
            accent="sage"
            selected={selected === 'local'}
            onClick={() => setSelected('local')}
            title={t('bootCheck.localRecommended')}
            description={t('bootCheck.localDescription')}
          />
        )}

        {/* Cloud option — always available; the only option on the web build */}
        <ModeCard
          testId="boot-check-mode-cloud"
          accent="primary"
          selected={selected === 'cloud'}
          onClick={() => setSelected('cloud')}
          title={t('bootCheck.cloudMode')}
          description={t('bootCheck.cloudDescription')}
        />
      </div>

      {/* Cloud URL + Token inputs — shown when cloud is selected */}
      {selected === 'cloud' && (
        <div className="mt-5 flex flex-col gap-3">
          <div className="flex flex-col gap-1">
            <label className="text-xs font-medium text-stone-700 dark:text-neutral-200">
              {t('bootCheck.coreRpcUrl')}
            </label>
            <input
              type="url"
              placeholder={t('bootCheck.rpcUrlPlaceholder')}
              value={cloudUrl}
              onChange={e => {
                const next = e.target.value;
                setCloudUrl(next);
                setUrlError(null);
                setUrlWarning(httpPublicHostWarning(next, t));
                setTestStatus({ kind: 'idle' });
              }}
              className="rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
            />
            {urlError && <p className="text-xs text-red-600">{urlError}</p>}
            {!urlError && urlWarning && (
              <p className="text-xs text-amber-600 dark:text-amber-500">{urlWarning}</p>
            )}
          </div>
          <div className="flex flex-col gap-1">
            <label className="text-xs font-medium text-stone-700 dark:text-neutral-200">
              {t('bootCheck.authToken')} (<code className="text-[10px]">OPENHUMAN_CORE_TOKEN</code>)
            </label>
            <input
              type="text"
              autoComplete="off"
              spellCheck={false}
              data-1p-ignore
              data-lpignore="true"
              placeholder={t('bootCheck.bearerTokenPlaceholder')}
              value={cloudToken}
              onChange={e => {
                setCloudToken(e.target.value);
                setTokenError(null);
                setTestStatus({ kind: 'idle' });
              }}
              className="rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
            />
            {tokenError && <p className="text-xs text-red-600">{tokenError}</p>}
            <p className="text-[11px] text-stone-500 dark:text-neutral-400 leading-snug">
              {t('bootCheck.storedLocally')} <code>Authorization: Bearer *** '</code>
              {t('bootCheck.rpcAuthSuffix')}
            </p>
          </div>

          <div className="flex items-center gap-3">
            <button
              type="button"
              onClick={handleTestConnection}
              disabled={testStatus.kind === 'testing'}
              className="rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-1.5 text-xs text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 disabled:opacity-60">
              {testStatus.kind === 'testing'
                ? t('bootCheck.testing')
                : t('bootCheck.testConnection')}
            </button>
            {testStatus.kind === 'ok' && (
              <span className="text-xs text-emerald-600" data-testid="test-status-ok">
                {t('bootCheck.connectedOk')}
              </span>
            )}
            {testStatus.kind === 'auth' && (
              <span className="text-xs text-red-600" data-testid="test-status-auth">
                {t('bootCheck.authFailed')}
              </span>
            )}
            {testStatus.kind === 'unreachable' && (
              <span className="text-xs text-red-600" data-testid="test-status-unreachable">
                {t('bootCheck.unreachablePrefix')} {testStatus.reason}
              </span>
            )}
          </div>
        </div>
      )}

      {/* Continue button — centered like onboarding */}
      <div className="mt-8 flex justify-center">
        <button
          type="button"
          onClick={handleContinue}
          className="rounded-xl bg-primary-500 px-8 py-3 text-sm font-semibold text-white hover:bg-primary-600 shadow-sm transition-colors">
          {t('common.continue')}
        </button>
      </div>
    </Panel>
  );
}

// ---------------------------------------------------------------------------
// Spinner / checking
// ---------------------------------------------------------------------------

function CheckingScreen() {
  const { t } = useT();
  return (
    <Panel>
      <div className="flex flex-col items-center gap-4 py-4">
        <div className="h-8 w-8 animate-spin rounded-full border-2 border-stone-300 dark:border-neutral-700 border-t-primary-500" />
        <p className="text-sm text-stone-600 dark:text-neutral-300">
          {t('bootCheck.checkingCore')}
        </p>
      </div>
    </Panel>
  );
}

// ---------------------------------------------------------------------------
// Result screens
// ---------------------------------------------------------------------------

interface ResultScreenProps {
  result: BootCheckResult;
  onRetry: () => void;
  onSwitchMode: () => void;
  onQuit: () => void;
  actionBusy: boolean;
  actionError: string | null;
  onAction: () => void;
}

function ResultScreen({
  result,
  onRetry,
  onSwitchMode,
  onQuit,
  actionBusy,
  actionError,
  onAction,
}: ResultScreenProps) {
  const { t } = useT();
  if (result.kind === 'match') return null;

  if (result.kind === 'unreachable') {
    const isPortConflict = result.portConflict === true;
    return (
      <Panel>
        <div className="flex flex-col items-center text-center">
          <h2 className="text-xl font-display text-stone-900 dark:text-neutral-100 mb-2">
            {isPortConflict ? t('bootCheck.portConflictTitle') : t('bootCheck.cannotReach')}
          </h2>
          <p className="text-sm text-stone-500 dark:text-neutral-400 max-w-md">
            {isPortConflict
              ? t('bootCheck.portConflictBody')
              : result.reason || t('bootCheck.cannotReachDesc')}
          </p>
        </div>
        {actionError && (
          <p className="mt-4 text-xs text-red-600 font-medium text-center">{actionError}</p>
        )}
        <div className="mt-6 flex justify-center gap-3 flex-wrap">
          {isPortConflict && (
            <button
              type="button"
              onClick={onAction}
              disabled={actionBusy}
              data-testid="fix-automatically-btn"
              className="rounded-xl bg-primary-500 px-5 py-2.5 text-sm font-semibold text-white hover:bg-primary-600 disabled:opacity-60">
              {actionBusy
                ? t('bootCheck.portConflictFixing')
                : t('bootCheck.portConflictFixButton')}
            </button>
          )}
          <button
            type="button"
            onClick={onRetry}
            disabled={actionBusy}
            className="rounded-xl border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-5 py-2.5 text-sm font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 disabled:opacity-60">
            {t('common.retry')}
          </button>
          <button
            type="button"
            onClick={onSwitchMode}
            disabled={actionBusy}
            className="rounded-xl border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-5 py-2.5 text-sm font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 disabled:opacity-60">
            {t('bootCheck.switchMode')}
          </button>
          <button
            type="button"
            onClick={onQuit}
            disabled={actionBusy}
            className="rounded-xl bg-red-600 px-5 py-2.5 text-sm font-semibold text-white hover:bg-red-700 disabled:opacity-60">
            {t('bootCheck.quit')}
          </button>
        </div>
      </Panel>
    );
  }

  if (result.kind === 'daemonDetected') {
    return (
      <Panel>
        <div className="flex flex-col items-center text-center">
          <h2 className="text-xl font-display text-stone-900 dark:text-neutral-100 mb-2">
            {t('bootCheck.legacyDetected')}
          </h2>
          <p className="text-sm text-stone-500 dark:text-neutral-400 max-w-md">
            {t('bootCheck.legacyDescription')}
          </p>
        </div>
        {actionError && (
          <p className="mt-4 text-xs text-red-600 font-medium text-center">{actionError}</p>
        )}
        <div className="mt-6 flex justify-center gap-3">
          <button
            type="button"
            onClick={onAction}
            disabled={actionBusy}
            className="rounded-xl bg-red-600 px-5 py-2.5 text-sm font-semibold text-white hover:bg-red-700 disabled:opacity-60">
            {actionBusy ? t('bootCheck.removing') : t('bootCheck.removeContinue')}
          </button>
          <button
            type="button"
            onClick={onSwitchMode}
            disabled={actionBusy}
            className="rounded-xl border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-5 py-2.5 text-sm font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 disabled:opacity-60">
            {t('bootCheck.switchMode')}
          </button>
        </div>
      </Panel>
    );
  }

  if (result.kind === 'outdatedLocal') {
    return (
      <Panel>
        <div className="flex flex-col items-center text-center">
          <h2 className="text-xl font-display text-stone-900 dark:text-neutral-100 mb-2">
            {t('bootCheck.localNeedsRestart')}
          </h2>
          <p className="text-sm text-stone-500 dark:text-neutral-400 max-w-md">
            {t('bootCheck.localNeedsRestartDesc')}
          </p>
        </div>
        {actionError && (
          <p className="mt-4 text-xs text-red-600 font-medium text-center">{actionError}</p>
        )}
        <div className="mt-6 flex justify-center gap-3">
          <button
            type="button"
            onClick={onAction}
            disabled={actionBusy}
            className="rounded-xl bg-primary-500 px-5 py-2.5 text-sm font-semibold text-white hover:bg-primary-600 disabled:opacity-60">
            {actionBusy ? t('bootCheck.restarting') : t('bootCheck.restartCore')}
          </button>
          <button
            type="button"
            onClick={onSwitchMode}
            disabled={actionBusy}
            className="rounded-xl border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-5 py-2.5 text-sm font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 disabled:opacity-60">
            {t('bootCheck.switchMode')}
          </button>
        </div>
      </Panel>
    );
  }

  if (result.kind === 'outdatedCloud') {
    return (
      <Panel>
        <div className="flex flex-col items-center text-center">
          <h2 className="text-xl font-display text-stone-900 dark:text-neutral-100 mb-2">
            {t('bootCheck.cloudNeedsUpdate')}
          </h2>
          <p className="text-sm text-stone-500 dark:text-neutral-400 max-w-md">
            {t('bootCheck.cloudNeedsUpdateDesc')}
          </p>
        </div>
        {actionError && (
          <p className="mt-4 text-xs text-red-600 font-medium text-center">{actionError}</p>
        )}
        <div className="mt-6 flex justify-center gap-3">
          <button
            type="button"
            onClick={onAction}
            disabled={actionBusy}
            className="rounded-xl bg-primary-500 px-5 py-2.5 text-sm font-semibold text-white hover:bg-primary-600 disabled:opacity-60">
            {actionBusy ? t('bootCheck.updating') : t('bootCheck.updateCloudCore')}
          </button>
          <button
            type="button"
            onClick={onSwitchMode}
            disabled={actionBusy}
            className="rounded-xl border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-5 py-2.5 text-sm font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 disabled:opacity-60">
            {t('bootCheck.switchMode')}
          </button>
        </div>
      </Panel>
    );
  }

  // noVersionMethod — treat like outdated, user picks which flavor of action
  return (
    <Panel>
      <div className="flex flex-col items-center text-center">
        <h2 className="text-xl font-display text-stone-900 dark:text-neutral-100 mb-2">
          {t('bootCheck.versionCheckFailed')}
        </h2>
        <p className="text-sm text-stone-500 dark:text-neutral-400 max-w-md">
          {t('bootCheck.versionCheckFailedDesc')}
        </p>
      </div>
      {actionError && (
        <p className="mt-4 text-xs text-red-600 font-medium text-center">{actionError}</p>
      )}
      <div className="mt-6 flex justify-center gap-3">
        <button
          type="button"
          onClick={onAction}
          disabled={actionBusy}
          className="rounded-xl bg-primary-500 px-5 py-2.5 text-sm font-semibold text-white hover:bg-primary-600 disabled:opacity-60">
          {actionBusy ? t('bootCheck.working') : t('bootCheck.restartUpdateCore')}
        </button>
        <button
          type="button"
          onClick={onSwitchMode}
          disabled={actionBusy}
          className="rounded-xl border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-5 py-2.5 text-sm font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-50 dark:hover:bg-neutral-800/60 disabled:opacity-60">
          {t('bootCheck.switchMode')}
        </button>
      </div>
    </Panel>
  );
}

// ---------------------------------------------------------------------------
// Main gate
// ---------------------------------------------------------------------------

interface BootCheckGateProps {
  children: React.ReactNode;
}

export default function BootCheckGate({ children }: BootCheckGateProps) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const coreMode = useAppSelector(state => state.coreMode.mode);

  const [phase, setPhase] = useState<Phase>(() =>
    coreMode.kind === 'unset' ? 'picker' : 'checking'
  );
  const [result, setResult] = useState<BootCheckResult | null>(null);
  const [actionBusy, setActionBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  // Prevent concurrent or stale runs.
  const runningRef = useRef(false);

  // Production transport lives in services/bootCheckService so direct
  // Tauri/RPC imports stay localized there.
  const transport = bootCheckTransport;

  const runCheck = useCallback(
    async (mode: CoreMode) => {
      if (runningRef.current) {
        log('[boot-check] gate — check already running, skipping duplicate');
        return;
      }
      runningRef.current = true;
      setPhase('checking');
      setResult(null);
      setActionError(null);
      log('[boot-check] gate — starting check mode=%s', mode.kind);

      try {
        const checkResult = await runBootCheck(mode, transport);
        log('[boot-check] gate — check result=%s', checkResult.kind);
        setPhase('result');
        setResult(checkResult);
      } catch (err) {
        logError('[boot-check] gate — unexpected error: %o', err);
        setPhase('result');
        setResult({
          kind: 'unreachable',
          reason: err instanceof Error ? err.message : t('bootCheck.unexpectedError'),
        });
      } finally {
        runningRef.current = false;
      }
    },
    // transport is stable (constructed inline but always same shape)
    // eslint-disable-next-line react-hooks/exhaustive-deps
    []
  );

  // Start check automatically when mode is set and we're in checking phase.
  useEffect(() => {
    if (coreMode.kind !== 'unset' && phase === 'checking') {
      void runCheck(coreMode);
    }
  }, [coreMode, phase, runCheck]);

  // ------------------------------------------------------------------
  // Picker confirm — dispatches setCoreMode and kicks off check.
  // ------------------------------------------------------------------
  const handlePickerConfirm = useCallback(
    (mode: CoreMode) => {
      log('[boot-check] gate — picker confirmed mode=%s', mode.kind);
      if (mode.kind === 'cloud') {
        storeRpcUrl(mode.url);
        storeCoreToken(mode.token ?? '');
        storeCoreMode('cloud');
      } else {
        storeRpcUrl('');
        clearStoredCoreToken();
        storeCoreMode('local');
      }
      clearCoreRpcUrlCache();
      clearCoreRpcTokenCache();
      dispatch(setCoreMode(mode));
      setPhase('checking');
    },
    [dispatch]
  );

  // ------------------------------------------------------------------
  // Switch mode — reset to picker.
  // ------------------------------------------------------------------
  const handleSwitchMode = useCallback(() => {
    log('[boot-check] gate — switch mode requested');
    storeRpcUrl('');
    clearStoredCoreToken();
    clearStoredCoreMode();
    clearCoreRpcUrlCache();
    clearCoreRpcTokenCache();
    dispatch(resetCoreMode());
    setPhase('picker');
    setResult(null);
    setActionError(null);
  }, [dispatch]);

  // ------------------------------------------------------------------
  // Quit the app.
  // ------------------------------------------------------------------
  const handleQuit = useCallback(async () => {
    log('[boot-check] gate — quit requested');
    try {
      await bootCheckTransport.invokeCmd('app_quit');
    } catch (err) {
      logError('[boot-check] gate — app_quit failed: %o', err);
    }
  }, []);

  // ------------------------------------------------------------------
  // Retry (unreachable state).
  // ------------------------------------------------------------------
  const handleRetry = useCallback(() => {
    log('[boot-check] gate — retry requested');
    if (coreMode.kind !== 'unset') {
      runCheck(coreMode);
    }
  }, [coreMode, runCheck]);

  // ------------------------------------------------------------------
  // Primary action per result kind.
  // ------------------------------------------------------------------
  const handleAction = useCallback(async () => {
    if (!result || actionBusy) return;
    setActionBusy(true);
    setActionError(null);

    try {
      if (result.kind === 'daemonDetected') {
        log('[boot-check] gate — removing legacy daemon');
        await transport.callRpc('openhuman.service_stop', {});
        await transport.callRpc('openhuman.service_uninstall', {});
        log('[boot-check] gate — daemon removed, re-running check');
      } else if (result.kind === 'outdatedLocal' || result.kind === 'noVersionMethod') {
        log('[boot-check] gate — restarting local core');
        await transport.invokeCmd('restart_core_process', {});
        log('[boot-check] gate — local core restarted');
      } else if (result.kind === 'outdatedCloud') {
        log('[boot-check] gate — triggering cloud core update');
        await transport.callRpc('openhuman.update_run', {});
        log('[boot-check] gate — cloud core update triggered');
      } else if (result.kind === 'unreachable' && result.portConflict) {
        log('[boot-check-gate] port conflict — invoking recover_port_conflict');
        const recovery = await recoverPortConflict();
        log(
          '[boot-check-gate] recovery result: success=%s message=%s',
          recovery.success,
          recovery.message
        );
        if (!recovery.success) {
          setActionError(t('bootCheck.portConflictFixFailed'));
          return;
        }
      }

      // Re-run the full check after the action.
      if (coreMode.kind !== 'unset') {
        runCheck(coreMode);
      }
    } catch (err) {
      logError('[boot-check] gate — action error: %o', err);
      setActionError(err instanceof Error ? err.message : t('bootCheck.actionFailed'));
    } finally {
      setActionBusy(false);
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [result, actionBusy, coreMode, runCheck]);

  // ------------------------------------------------------------------
  // Render
  // ------------------------------------------------------------------

  if (phase === 'picker' || coreMode.kind === 'unset') {
    return (
      <>
        <ModePicker onConfirm={handlePickerConfirm} />
      </>
    );
  }

  if (phase === 'checking') {
    return <CheckingScreen />;
  }

  if (result?.kind === 'match') {
    return <>{children}</>;
  }

  return (
    <>
      <ResultScreen
        result={result ?? { kind: 'unreachable', reason: 'Unknown error' }}
        onRetry={handleRetry}
        onSwitchMode={handleSwitchMode}
        onQuit={handleQuit}
        actionBusy={actionBusy}
        actionError={actionError}
        onAction={handleAction}
      />
    </>
  );
}
