import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { callCoreRpc } from '../../../services/coreRpcClient';
import { openUrl } from '../../../utils/openUrl';
import { isTauri } from '../../../utils/tauriCommands/common';

// Connect/disconnect surface for OpenAI "Sign in with ChatGPT" (inference
// OAuth). The Rust core owns the flow via the `openhuman.inference_openai_oauth_*`
// RPCs; this component only drives them and reflects status. The same flow
// already ships in onboarding (`ApiKeysStep`); this brings it to AI settings so
// users can connect, re-check, or disconnect after onboarding.
//
// The flow is intentionally two-step (start -> user signs in -> paste the
// loopback redirect URL -> complete) because the system browser redirects to a
// local `http://127.0.0.1:1455/` callback the desktop app cannot read directly.

type OpenAiOAuthStatus = { connected: boolean; authMethod?: string | null };

interface OpenAiOAuthConnectProps {
  /** Prefix for `data-testid`s so onboarding and settings stay distinguishable. */
  testIdPrefix?: string;
  /** Notified whenever the connected state is established or cleared. */
  onConnectedChange?: (connected: boolean) => void;
  /** When true, expose a "Disconnect" control once connected. */
  allowDisconnect?: boolean;
}

const OpenAiOAuthConnect = ({
  testIdPrefix = 'openai-oauth',
  onConnectedChange,
  allowDisconnect = false,
}: OpenAiOAuthConnectProps) => {
  const { t } = useT();
  const [connected, setConnected] = useState(false);
  const [busy, setBusy] = useState(false);
  const [awaitingCallback, setAwaitingCallback] = useState(false);
  const [callbackUrl, setCallbackUrl] = useState('');
  const [error, setError] = useState<string | null>(null);

  const applyConnected = useCallback(
    (next: boolean) => {
      setConnected(next);
      onConnectedChange?.(next);
    },
    [onConnectedChange]
  );

  const refreshStatus = useCallback(async () => {
    if (!isTauri()) {
      return;
    }
    try {
      const res = await callCoreRpc<{ result: OpenAiOAuthStatus }>({
        method: 'openhuman.inference_openai_oauth_status',
        params: {},
      });
      applyConnected(Boolean(res?.result?.connected));
    } catch (err) {
      // Status is best-effort; a failed probe must not block the connect UI.
      console.debug('[ai-settings:openai-oauth] status check failed', err);
    }
  }, [applyConnected]);

  useEffect(() => {
    void refreshStatus();
  }, [refreshStatus]);

  const handleStart = async () => {
    if (!isTauri()) {
      setError(t('settings.ai.openaiOauthDesktopOnly'));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const res = await callCoreRpc<{ result: { authUrl: string } }>({
        method: 'openhuman.inference_openai_oauth_start',
        params: {},
      });
      const authUrl = res?.result?.authUrl?.trim();
      if (!authUrl) {
        throw new Error('missing authUrl');
      }
      setAwaitingCallback(true);
      await openUrl(authUrl);
    } catch (err) {
      console.warn('[ai-settings:openai-oauth] start failed', err);
      setError(t('settings.ai.openaiOauthStartError'));
    } finally {
      setBusy(false);
    }
  };

  const handleComplete = async () => {
    const callback = callbackUrl.trim();
    if (!callback) {
      setError(t('settings.ai.openaiOauthCallbackRequired'));
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await callCoreRpc({
        method: 'openhuman.inference_openai_oauth_complete',
        params: { callback_url: callback },
      });
      setCallbackUrl('');
      setAwaitingCallback(false);
      applyConnected(true);
    } catch (err) {
      console.warn('[ai-settings:openai-oauth] complete failed', err);
      setError(t('settings.ai.openaiOauthCompleteError'));
    } finally {
      setBusy(false);
    }
  };

  const handleDisconnect = async () => {
    setBusy(true);
    setError(null);
    try {
      await callCoreRpc({ method: 'openhuman.inference_openai_oauth_disconnect', params: {} });
      setAwaitingCallback(false);
      setCallbackUrl('');
      applyConnected(false);
    } catch (err) {
      console.warn('[ai-settings:openai-oauth] disconnect failed', err);
      setError(t('settings.ai.openaiOauthDisconnectError'));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div
      data-testid={`${testIdPrefix}-section`}
      className="flex flex-col gap-2 rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/50 p-3">
      <div className="flex flex-wrap items-center justify-between gap-2">
        <span className="text-[11px] font-semibold uppercase tracking-wide text-stone-500 dark:text-neutral-400">
          {t('settings.ai.openaiOauthTitle')}
        </span>
        {connected ? (
          <span
            data-testid={`${testIdPrefix}-connected`}
            className="text-xs font-medium text-sage-700 dark:text-sage-300">
            {t('settings.ai.openaiOauthConnected')}
          </span>
        ) : null}
      </div>
      <p className="text-xs text-stone-500 dark:text-neutral-400">
        {t('settings.ai.openaiOauthDescription')}
      </p>

      {connected ? (
        allowDisconnect ? (
          <button
            type="button"
            data-testid={`${testIdPrefix}-disconnect`}
            disabled={busy}
            onClick={() => void handleDisconnect()}
            className="self-start rounded-lg border border-stone-200 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-xs font-medium text-stone-700 dark:text-neutral-200 hover:bg-stone-100 dark:hover:bg-neutral-800 disabled:opacity-50">
            {t('settings.ai.openaiOauthDisconnect')}
          </button>
        ) : null
      ) : (
        <>
          <button
            type="button"
            data-testid={`${testIdPrefix}-connect`}
            disabled={busy}
            onClick={() => void handleStart()}
            className="self-start rounded-lg border border-primary-500 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-sm font-medium text-primary-700 dark:text-primary-300 hover:bg-primary-100 dark:hover:bg-primary-500/20 disabled:opacity-50">
            {busy ? t('settings.ai.openaiOauthOpening') : t('settings.ai.openaiOauthConnect')}
          </button>
          {awaitingCallback ? (
            <div className="flex flex-col gap-1.5">
              <p className="text-[11px] text-stone-500 dark:text-neutral-400">
                {t('settings.ai.openaiOauthCallbackHint')}
              </p>
              <input
                data-testid={`${testIdPrefix}-callback-input`}
                type="text"
                autoComplete="off"
                spellCheck={false}
                placeholder={t('settings.ai.openaiOauthCallbackPlaceholder')}
                value={callbackUrl}
                onChange={e => {
                  setCallbackUrl(e.target.value);
                  setError(null);
                }}
                className="rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-xs text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
              />
              <button
                type="button"
                data-testid={`${testIdPrefix}-complete`}
                disabled={busy}
                onClick={() => void handleComplete()}
                className="self-start text-xs font-medium text-primary-600 dark:text-primary-400 underline disabled:opacity-50">
                {t('settings.ai.openaiOauthFinish')}
              </button>
            </div>
          ) : null}
        </>
      )}

      {error ? (
        <p data-testid={`${testIdPrefix}-error`} className="text-xs font-medium text-red-600">
          {error}
        </p>
      ) : null}
    </div>
  );
};

export default OpenAiOAuthConnect;
