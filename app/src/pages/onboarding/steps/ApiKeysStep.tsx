import { useCallback, useEffect, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { setCloudProviderKey } from '../../../services/api/aiSettingsApi';
import { callCoreRpc } from '../../../services/coreRpcClient';
import { openUrl } from '../../../utils/openUrl';
import { isTauri } from '../../../utils/tauriCommands/common';
import OnboardingNextButton from '../components/OnboardingNextButton';

interface ApiKeysStepProps {
  onNext: () => void;
  onSkip: () => void;
}

type OpenAiOAuthStatus = { connected: boolean; authMethod?: string | null };

// Fixed OpenAI OAuth loopback origin (mirrors `REDIRECT_URI` in
// src/openhuman/inference/openai_oauth/config.rs). Interpolated into the
// translated callback hint via the `{url}` placeholder so the literal lives in
// one place instead of inside all 14 locale strings.
const OPENAI_OAUTH_CALLBACK_ORIGIN = 'http://127.0.0.1:1455/';
// Non-display technical sentinel (example redirect URL shown as the input
// placeholder) — exempt from i18n.
const OPENAI_OAUTH_CALLBACK_PLACEHOLDER = `${OPENAI_OAUTH_CALLBACK_ORIGIN}auth/callback?code=...&state=...`;

const ApiKeysStep = ({ onNext, onSkip }: ApiKeysStepProps) => {
  const { t } = useT();
  const [openai, setOpenai] = useState('');
  const [anthropic, setAnthropic] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [oauthConnected, setOauthConnected] = useState(false);
  const [oauthBusy, setOauthBusy] = useState(false);
  const [oauthAwaitingCallback, setOauthAwaitingCallback] = useState(false);
  const [oauthCallbackUrl, setOauthCallbackUrl] = useState('');

  const refreshOAuthStatus = useCallback(async () => {
    if (!isTauri()) {
      return;
    }
    try {
      const res = await callCoreRpc<{ result: OpenAiOAuthStatus }>({
        method: 'openhuman.inference_openai_oauth_status',
        params: {},
      });
      setOauthConnected(Boolean(res?.result?.connected));
    } catch (err) {
      console.debug('[onboarding:api-keys] oauth status check failed', err);
    }
  }, []);

  useEffect(() => {
    void refreshOAuthStatus();
  }, [refreshOAuthStatus]);

  const handleOpenAiOAuthStart = async () => {
    if (!isTauri()) {
      setError(t('onboarding.apiKeys.oauthDesktopOnly'));
      return;
    }
    setOauthBusy(true);
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
      setOauthAwaitingCallback(true);
      await openUrl(authUrl);
    } catch (err) {
      console.warn('[onboarding:api-keys] oauth start failed', err);
      setError(t('onboarding.apiKeys.oauthStartFailed'));
    } finally {
      setOauthBusy(false);
    }
  };

  const handleOpenAiOAuthComplete = async () => {
    const callback = oauthCallbackUrl.trim();
    if (!callback) {
      setError(t('onboarding.apiKeys.oauthPasteRedirect'));
      return;
    }
    setOauthBusy(true);
    setError(null);
    try {
      await callCoreRpc({
        method: 'openhuman.inference_openai_oauth_complete',
        params: { callback_url: callback },
      });
      setOauthCallbackUrl('');
      setOauthAwaitingCallback(false);
      setOauthConnected(true);
    } catch (err) {
      console.warn('[onboarding:api-keys] oauth complete failed', err);
      setError(t('onboarding.apiKeys.oauthCompleteFailed'));
    } finally {
      setOauthBusy(false);
    }
  };

  const handleSave = async () => {
    const trimmedOpenai = openai.trim();
    const trimmedAnthropic = anthropic.trim();
    if (!trimmedOpenai && !trimmedAnthropic && !oauthConnected) {
      onSkip();
      return;
    }

    setSaving(true);
    setError(null);
    try {
      if (trimmedOpenai) {
        await setCloudProviderKey('openai', trimmedOpenai);
      }
      if (trimmedAnthropic) {
        await setCloudProviderKey('anthropic', trimmedAnthropic);
      }
      onNext();
    } catch (err) {
      console.warn('[onboarding:api-keys] save failed', err);
      setError(t('onboarding.apiKeys.saveError'));
    } finally {
      setSaving(false);
    }
  };

  return (
    <div
      data-testid="onboarding-api-keys-step"
      className="rounded-2xl bg-white dark:bg-neutral-900 p-10 shadow-soft animate-fade-up">
      <div className="text-center">
        <h1 className="text-2xl font-display text-stone-900 dark:text-neutral-100 mb-2 leading-tight">
          {t('onboarding.apiKeys.title')}
        </h1>
        <p className="text-stone-500 dark:text-neutral-400 text-sm leading-relaxed">
          {t('onboarding.apiKeys.subtitle')}
        </p>
      </div>

      <div className="mt-6 flex flex-col gap-4">
        <div className="flex flex-col gap-2 rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/40 p-3">
          <div className="flex flex-wrap items-center justify-between gap-2">
            <span className="text-xs font-medium text-stone-700 dark:text-neutral-200">
              {t('onboarding.apiKeys.openaiLabel')}
            </span>
            {oauthConnected ? (
              <span
                data-testid="onboarding-openai-oauth-connected"
                className="text-xs font-medium text-sage-700 dark:text-sage-300">
                {t('onboarding.apiKeys.openaiOauthConnected')}
              </span>
            ) : null}
          </div>
          <p className="text-[11px] text-stone-500 dark:text-neutral-400">
            {t('onboarding.apiKeys.openaiOauthHint')}
          </p>
          <button
            type="button"
            data-testid="onboarding-openai-oauth-connect"
            disabled={oauthBusy || oauthConnected || saving}
            onClick={() => void handleOpenAiOAuthStart()}
            className="rounded-lg border border-primary-500 bg-primary-50 dark:bg-primary-500/10 px-3 py-2 text-sm font-medium text-primary-700 dark:text-primary-300 hover:bg-primary-100 dark:hover:bg-primary-500/20 disabled:opacity-50">
            {oauthBusy
              ? t('onboarding.apiKeys.openaiOauthOpening')
              : t('onboarding.apiKeys.openaiOauthConnect')}
          </button>
          {oauthAwaitingCallback && !oauthConnected ? (
            <div className="flex flex-col gap-1.5">
              <p className="text-[11px] text-stone-500 dark:text-neutral-400">
                {t('onboarding.apiKeys.openaiOauthCallbackHint').replace(
                  '{url}',
                  OPENAI_OAUTH_CALLBACK_ORIGIN
                )}
              </p>
              <input
                data-testid="onboarding-openai-oauth-callback-input"
                type="text"
                autoComplete="off"
                spellCheck={false}
                placeholder={OPENAI_OAUTH_CALLBACK_PLACEHOLDER}
                value={oauthCallbackUrl}
                onChange={e => {
                  setOauthCallbackUrl(e.target.value);
                  setError(null);
                }}
                className="rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-xs text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
              />
              <button
                type="button"
                data-testid="onboarding-openai-oauth-complete"
                disabled={oauthBusy || saving}
                onClick={() => void handleOpenAiOAuthComplete()}
                className="self-start text-xs font-medium text-primary-600 dark:text-primary-400 underline disabled:opacity-50">
                {t('onboarding.apiKeys.finishSignIn')}
              </button>
            </div>
          ) : null}
          <div className="relative flex items-center gap-2 py-1">
            <div className="h-px flex-1 bg-stone-200 dark:bg-neutral-700" />
            <span className="text-[10px] uppercase tracking-wide text-stone-400 dark:text-neutral-500">
              {t('onboarding.apiKeys.orApiKey')}
            </span>
            <div className="h-px flex-1 bg-stone-200 dark:bg-neutral-700" />
          </div>
          <input
            id="onboarding-openai-key"
            data-testid="onboarding-api-keys-openai-input"
            type="password"
            autoComplete="off"
            spellCheck={false}
            placeholder={t('onboarding.apiKeys.openaiPlaceholder')}
            value={openai}
            onChange={e => {
              setOpenai(e.target.value);
              setError(null);
            }}
            className="rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
          />
        </div>

        <div className="flex flex-col gap-1.5">
          <label
            htmlFor="onboarding-anthropic-key"
            className="text-xs font-medium text-stone-700 dark:text-neutral-200">
            {t('onboarding.apiKeys.anthropicLabel')}
          </label>
          <input
            id="onboarding-anthropic-key"
            data-testid="onboarding-api-keys-anthropic-input"
            type="password"
            autoComplete="off"
            spellCheck={false}
            placeholder={t('onboarding.apiKeys.anthropicPlaceholder')}
            value={anthropic}
            onChange={e => {
              setAnthropic(e.target.value);
              setError(null);
            }}
            className="rounded-lg border border-stone-300 dark:border-neutral-700 bg-white dark:bg-neutral-900 px-3 py-2 text-sm text-stone-900 dark:text-neutral-100 placeholder-stone-400 dark:placeholder-neutral-500 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
          />
        </div>

        {error ? <p className="text-xs font-medium text-red-600">{error}</p> : null}
      </div>

      <div className="mt-8">
        <OnboardingNextButton
          label={t('onboarding.apiKeys.continue')}
          loading={saving}
          loadingLabel={t('onboarding.apiKeys.saving')}
          onClick={() => void handleSave()}
        />
      </div>

      <div className="mt-4 flex justify-center">
        <button
          type="button"
          onClick={onSkip}
          disabled={saving}
          className="text-xs text-stone-500 dark:text-neutral-400 hover:text-stone-700 dark:hover:text-neutral-200 underline disabled:opacity-50">
          {t('onboarding.apiKeys.skipForNow')}
        </button>
      </div>
    </div>
  );
};

export default ApiKeysStep;
