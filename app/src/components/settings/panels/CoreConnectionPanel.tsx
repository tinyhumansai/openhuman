/**
 * CoreConnectionPanel — Settings → Core connection.
 *
 * Promotes the pre-existing "cloud" core mode (a persisted remote-core RPC
 * URL + bearer token, previously reachable only from the pre-router
 * BootCheckGate picker) into a first-class, in-app setting, and adds a live
 * connect/failure status indicator.
 *
 * This deliberately reuses the existing cloud-mode plumbing — the `coreMode`
 * Redux slice, `configPersistence` storage keys, and
 * `testCoreRpcConnection` — rather than introducing a second remote-core
 * mechanism. The shell-level env-var attach path (`OPENHUMAN_CORE_REUSE_EXISTING`,
 * `OPENHUMAN_CORE_TOKEN`) is intentionally left as a documented dev-only
 * override and is not surfaced here (GH-4396).
 *
 * Boot-gate hard-fail/fallback semantics are unchanged: switching mode here
 * persists the choice and restarts the app so the normal BootCheckGate flow
 * re-runs against the new mode. This panel only *surfaces* connection state;
 * it does not change what happens when a configured core is unreachable.
 */
import { invoke } from '@tauri-apps/api/core';
import debug from 'debug';
import { useCallback, useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  clearCoreRpcTokenCache,
  clearCoreRpcUrlCache,
  testCoreRpcConnection,
} from '../../../services/coreRpcClient';
import { type CoreMode, setCoreMode } from '../../../store/coreModeSlice';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { CORE_RPC_URL } from '../../../utils/config';
import {
  clearStoredCoreToken,
  isLocalOrPrivateNetworkHost,
  isTauriEnvironment,
  normalizeRpcUrl,
  storeCoreMode,
  storeCoreToken,
  storeRpcUrl,
} from '../../../utils/configPersistence';
import { restartApp } from '../../../utils/tauriCommands/core';
import Button from '../../ui/Button';
import { SettingsRow, SettingsSection, SettingsSwitch, SettingsTextField } from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

const log = debug('settings:core');

/** Live reachability of the currently-active core. */
type LiveStatus =
  | { kind: 'checking' }
  | { kind: 'connected' }
  | { kind: 'authFailed' }
  | { kind: 'unreachable'; reason: string };

/** Result of a one-shot "Test connection" against the typed remote inputs. */
type TestStatus =
  | { kind: 'idle' }
  | { kind: 'testing' }
  | { kind: 'ok' }
  | { kind: 'auth' }
  | { kind: 'unreachable'; reason: string };

/**
 * Resolve the URL the active core is actually reachable at. Cloud mode stores
 * the user's chosen URL in Redux; local mode picks a dynamic port at launch,
 * so the authoritative value lives in the Tauri shell (`core_rpc_url`).
 */
async function resolveActiveCoreUrl(coreMode: CoreMode): Promise<string | null> {
  if (coreMode.kind === 'cloud') return coreMode.url;
  if (!isTauriEnvironment()) return CORE_RPC_URL;
  try {
    return await invoke<string>('core_rpc_url');
  } catch (err) {
    log('resolveActiveCoreUrl: core_rpc_url invoke failed: %o', err);
    return null;
  }
}

const CONNECTION_TEST_TIMEOUT_MS = 10_000;

/**
 * `testCoreRpcConnection` with a bounded deadline. `testCoreRpcConnection`
 * supports an `AbortSignal` but the callers didn't pass one, so a non-responsive
 * endpoint left the live status / Test button stuck in `checking`/`testing`
 * until the platform socket timeout (minutes). Abort after
 * CONNECTION_TEST_TIMEOUT_MS so the UI resolves to "unreachable" promptly.
 */
async function testCoreRpcConnectionWithTimeout(url: string, token?: string): Promise<Response> {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), CONNECTION_TEST_TIMEOUT_MS);
  try {
    return await testCoreRpcConnection(url, token, { signal: controller.signal });
  } finally {
    clearTimeout(timer);
  }
}

const CoreConnectionPanel = () => {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const coreMode = useAppSelector(state => state.coreMode.mode);
  // A non-Tauri web build cannot start a local core (no `start_core_process`),
  // so the boot picker forces cloud mode there; keep that invariant here so a
  // web user can't persist `local` and brick the next boot.
  const canUseLocal = isTauriEnvironment();

  // ── Editable form state ────────────────────────────────────────────────
  // Seeded from the persisted cloud-mode config so the panel reflects the
  // current setting on open.
  const [useRemote, setUseRemote] = useState(!canUseLocal || coreMode.kind === 'cloud');
  const [url, setUrl] = useState(coreMode.kind === 'cloud' ? coreMode.url : '');
  const [token, setToken] = useState(coreMode.kind === 'cloud' ? (coreMode.token ?? '') : '');
  const [formError, setFormError] = useState<string | null>(null);
  const [testStatus, setTestStatus] = useState<TestStatus>({ kind: 'idle' });
  const [saving, setSaving] = useState(false);
  const [showToken, setShowToken] = useState(false);

  // ── Live status indicator (against the currently-active core) ───────────
  const [liveStatus, setLiveStatus] = useState<LiveStatus>({ kind: 'checking' });
  const [activeUrl, setActiveUrl] = useState<string | null>(null);
  const checkSeq = useRef(0);

  const runLiveCheck = useCallback(async () => {
    const seq = ++checkSeq.current;
    setLiveStatus({ kind: 'checking' });
    log('runLiveCheck: mode=%s', coreMode.kind);
    const resolved = await resolveActiveCoreUrl(coreMode);
    if (seq !== checkSeq.current) return; // superseded by a newer check
    setActiveUrl(resolved);
    if (!resolved) {
      setLiveStatus({ kind: 'unreachable', reason: t('settings.about.serverUrlUnavailable') });
      return;
    }
    try {
      const response = await testCoreRpcConnectionWithTimeout(resolved);
      if (seq !== checkSeq.current) return;
      if (response.status === 401 || response.status === 403) {
        log('runLiveCheck: auth failed (status=%d)', response.status);
        setLiveStatus({ kind: 'authFailed' });
        return;
      }
      if (!response.ok) {
        log('runLiveCheck: HTTP %d', response.status);
        setLiveStatus({ kind: 'unreachable', reason: `HTTP ${response.status}` });
        return;
      }
      // Drain the body so the connection can be reused; a JSON-RPC error body
      // on a 200 does not disprove reachability.
      try {
        await response.json();
      } catch {
        /* non-JSON body is unusual but still reachable */
      }
      log('runLiveCheck: connected');
      setLiveStatus({ kind: 'connected' });
    } catch (err) {
      if (seq !== checkSeq.current) return;
      const reason = err instanceof Error ? err.message : 'Connection failed';
      log('runLiveCheck: errored: %o', err);
      setLiveStatus({ kind: 'unreachable', reason });
    }
  }, [coreMode, t]);

  useEffect(() => {
    // runLiveCheck flips the status to `checking` synchronously; that is the
    // intended entry transition for the live probe (also used by Recheck), not
    // a cascading render.
    // eslint-disable-next-line react-hooks/set-state-in-effect
    void runLiveCheck();
  }, [runLiveCheck]);

  // ── Validation (mirrors the BootCheckGate cloud picker) ─────────────────
  const validate = (): { url: string; token: string } | null => {
    const rawUrl = url.trim();
    if (!rawUrl) {
      setFormError(t('bootCheck.invalidUrl'));
      return null;
    }
    const normalized = normalizeRpcUrl(rawUrl);
    try {
      const parsed = new URL(normalized);
      if (parsed.protocol !== 'http:' && parsed.protocol !== 'https:') {
        setFormError(t('bootCheck.urlMustStartWith'));
        return null;
      }
      // The separate token field is the only credential path; a
      // `user:pass@host` URL would be persisted and echoed back in the active-URL
      // description, leaking a secret. Reject it.
      if (parsed.username || parsed.password) {
        setFormError(t('bootCheck.validUrlRequired'));
        return null;
      }
    } catch {
      setFormError(t('bootCheck.validUrlRequired'));
      return null;
    }
    const trimmedToken = token.trim();
    if (!trimmedToken) {
      setFormError(t('bootCheck.tokenRequired'));
      return null;
    }
    setFormError(null);
    return { url: normalized, token: trimmedToken };
  };

  const httpWarning = (() => {
    if (!useRemote) return null;
    const trimmed = url.trim();
    if (!trimmed) return null;
    try {
      const parsed = new URL(normalizeRpcUrl(trimmed));
      if (parsed.protocol === 'http:' && !isLocalOrPrivateNetworkHost(parsed.hostname)) {
        return t('bootCheck.httpPublicWarning');
      }
    } catch {
      /* validate() surfaces parse errors on save */
    }
    return null;
  })();

  const handleTest = async () => {
    const validated = validate();
    if (!validated) return;
    setTestStatus({ kind: 'testing' });
    log('handleTest: url=%s tokenLen=%d', validated.url, validated.token.length);
    try {
      const response = await testCoreRpcConnectionWithTimeout(validated.url, validated.token);
      if (response.status === 401 || response.status === 403) {
        setTestStatus({ kind: 'auth' });
        return;
      }
      if (!response.ok) {
        setTestStatus({ kind: 'unreachable', reason: `HTTP ${response.status}` });
        return;
      }
      try {
        await response.json();
      } catch {
        /* reachable regardless of body shape */
      }
      setTestStatus({ kind: 'ok' });
    } catch (err) {
      const reason = err instanceof Error ? err.message : 'Connection failed';
      setTestStatus({ kind: 'unreachable', reason });
    }
  };

  // ── Dirty detection ─────────────────────────────────────────────────────
  // Enable Save only when the desired mode differs from the persisted one.
  const isDirty = (() => {
    if (!useRemote) return coreMode.kind !== 'local';
    if (coreMode.kind !== 'cloud') return true;
    return (
      normalizeRpcUrl(url.trim() || '') !== coreMode.url || token.trim() !== (coreMode.token ?? '')
    );
  })();

  const handleSave = async () => {
    if (saving) return;
    if (useRemote) {
      const validated = validate();
      if (!validated) return;
      log(
        'handleSave: switching to remote core url=%s tokenLen=%d',
        validated.url,
        validated.token.length
      );
      setSaving(true);
      // NOTE: the bearer is persisted in plain localStorage via storeCoreToken,
      // matching the existing cloud-mode picker. A renderer XSS could read it
      // (security audit U3). Migrating this to the OS keychain is a known
      // follow-up tracked with the rest of cloud-mode token storage; this panel
      // intentionally does not block on it (GH-4396 scope decision).
      storeRpcUrl(validated.url);
      storeCoreToken(validated.token);
      storeCoreMode('cloud');
      clearCoreRpcUrlCache();
      clearCoreRpcTokenCache();
      dispatch(setCoreMode({ kind: 'cloud', url: validated.url, token: validated.token }));
    } else {
      log('handleSave: switching to local core');
      setSaving(true);
      storeRpcUrl('');
      clearStoredCoreToken();
      storeCoreMode('local');
      clearCoreRpcUrlCache();
      clearCoreRpcTokenCache();
      dispatch(setCoreMode({ kind: 'local' }));
    }
    // Restart so BootCheckGate re-runs against the new mode (unchanged
    // boot-gate semantics). In dev this is a renderer reload. The mode is
    // already persisted + dispatched above, so on restart failure recover the
    // button instead of wedging it in `saving` forever.
    try {
      await restartApp();
    } catch (err) {
      log('handleSave: restartApp failed: %o', err);
      setSaving(false);
      setFormError(t('common.error'));
    }
  };

  // ── Live status rendering ───────────────────────────────────────────────
  const statusText = (() => {
    switch (liveStatus.kind) {
      case 'checking':
        return t('settings.core.statusChecking');
      case 'connected':
        return coreMode.kind === 'cloud'
          ? t('settings.core.statusConnectedRemote')
          : t('settings.core.statusConnectedLocal');
      case 'authFailed':
        return t('settings.core.statusAuthFailed');
      case 'unreachable':
        return `${t('settings.core.statusUnreachable')} — ${liveStatus.reason}`;
    }
  })();

  const statusDotClass = (() => {
    switch (liveStatus.kind) {
      case 'connected':
        return 'bg-sage-500';
      case 'checking':
        return 'bg-amber-400 animate-pulse';
      default:
        return 'bg-coral-500';
    }
  })();

  return (
    <SettingsPanel description={t('settings.core.menuDesc')}>
      {/* Live status indicator */}
      <SettingsSection title={t('settings.about.connection')}>
        <SettingsRow
          label={statusText}
          description={activeUrl ?? undefined}
          control={
            <div className="flex items-center gap-2">
              <span
                className={`inline-block h-2.5 w-2.5 rounded-full flex-shrink-0 ${statusDotClass}`}
                aria-hidden="true"
                data-testid="core-status-dot"
              />
              <Button
                variant="secondary"
                size="xs"
                onClick={() => void runLiveCheck()}
                disabled={liveStatus.kind === 'checking'}>
                {t('settings.core.recheck')}
              </Button>
            </div>
          }
        />
        <div className="px-4 py-3" data-testid="core-status-text">
          <p className="text-[11px] text-content-muted leading-relaxed">
            {coreMode.kind === 'cloud'
              ? t('settings.about.connectionHelperCloud')
              : t('settings.about.connectionHelperLocal')}
          </p>
        </div>
      </SettingsSection>

      {/* Remote-core toggle + config */}
      <SettingsSection>
        <SettingsRow
          label={t('settings.core.useRemoteToggle')}
          description={t('settings.core.useRemoteToggleDesc')}
          control={
            <SettingsSwitch
              id="core-use-remote"
              checked={useRemote}
              disabled={!canUseLocal}
              onCheckedChange={next => {
                // Web builds can't run a local core — refuse to switch remote off.
                if (!canUseLocal && !next) return;
                setUseRemote(next);
                setTestStatus({ kind: 'idle' });
                setFormError(null);
              }}
              aria-label={t('settings.core.useRemoteToggle')}
              data-testid="core-use-remote-toggle"
            />
          }
        />

        {useRemote && (
          <div className="flex flex-col gap-3 px-4 py-4">
            <div className="flex flex-col gap-1">
              <label
                htmlFor="core-remote-url"
                className="text-xs font-medium text-content-secondary">
                {t('bootCheck.coreRpcUrl')}
              </label>
              <SettingsTextField
                id="core-remote-url"
                type="url"
                placeholder={t('bootCheck.rpcUrlPlaceholder')}
                value={url}
                onChange={e => {
                  setUrl(e.target.value);
                  setFormError(null);
                  setTestStatus({ kind: 'idle' });
                }}
              />
              {httpWarning && (
                <p className="text-xs text-amber-600 dark:text-amber-500">{httpWarning}</p>
              )}
            </div>

            <div className="flex flex-col gap-1">
              <div className="flex items-center justify-between">
                <label
                  htmlFor="core-remote-token"
                  className="text-xs font-medium text-content-secondary">
                  {t('bootCheck.authToken')} (
                  <code className="text-[10px]">OPENHUMAN_CORE_TOKEN</code>)
                </label>
                <button
                  type="button"
                  className="text-[11px] text-content-muted hover:text-content-secondary"
                  onClick={() => setShowToken(s => !s)}
                  data-testid="core-token-reveal">
                  {showToken ? t('settings.search.hide') : t('settings.search.show')}
                </button>
              </div>
              <SettingsTextField
                id="core-remote-token"
                type={showToken ? 'text' : 'password'}
                mono
                autoComplete="off"
                spellCheck={false}
                data-1p-ignore
                data-lpignore="true"
                placeholder={t('bootCheck.bearerTokenPlaceholder')}
                value={token}
                onChange={e => {
                  setToken(e.target.value);
                  setFormError(null);
                  setTestStatus({ kind: 'idle' });
                }}
              />
              <p className="text-[11px] text-content-muted leading-snug">
                {t('bootCheck.storedLocally')} <code>Authorization: Bearer …</code>{' '}
                {t('bootCheck.rpcAuthSuffix')}
              </p>
            </div>

            <div className="flex items-center gap-3">
              <Button
                variant="secondary"
                size="sm"
                onClick={handleTest}
                disabled={testStatus.kind === 'testing'}>
                {testStatus.kind === 'testing'
                  ? t('bootCheck.testing')
                  : t('bootCheck.testConnection')}
              </Button>
              {testStatus.kind === 'ok' && (
                <span className="text-xs text-sage-600" data-testid="core-test-ok">
                  {t('bootCheck.connectedOk')}
                </span>
              )}
              {testStatus.kind === 'auth' && (
                <span className="text-xs text-coral-600" data-testid="core-test-auth">
                  {t('bootCheck.authFailed')}
                </span>
              )}
              {testStatus.kind === 'unreachable' && (
                <span className="text-xs text-coral-600" data-testid="core-test-unreachable">
                  {t('bootCheck.unreachablePrefix')} {testStatus.reason}
                </span>
              )}
            </div>
          </div>
        )}

        {formError && <p className="px-4 pb-2 text-xs text-coral-600">{formError}</p>}

        <div className="flex items-center justify-between gap-3 px-4 py-3">
          <p className="text-[11px] text-content-muted leading-snug">
            {t('settings.core.applyRestartNote')}
          </p>
          <Button onClick={handleSave} disabled={!isDirty || saving} data-testid="core-save-btn">
            {t('settings.core.save')}
          </Button>
        </div>
      </SettingsSection>
    </SettingsPanel>
  );
};

export default CoreConnectionPanel;
