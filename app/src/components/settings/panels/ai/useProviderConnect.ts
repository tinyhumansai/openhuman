/*
 * Provider-connect orchestration: writing a credential (API key / endpoint /
 * OAuth / CLI login), live-probing it, and rolling back on failure. Shared by
 * the built-in cloud provider chips, the local-runtime chips, and the Codex
 * connect button.
 */
import { useCallback, useEffect, useState } from 'react';

import {
  classifyProviderVerificationFailure,
  clearCloudProviderKey,
  describeProviderVerificationFailure,
  flushCloudProviders,
  importOpenAiCodexCliAuth,
  listProviderModels,
  loadProviderAuthErrors,
  OPENAI_CODEX_OAUTH_MISSING_AUTH_URL,
  OPENAI_CODEX_OAUTH_MISSING_CALLBACK_URL,
  type ProviderAuthError,
  setCloudProviderKey,
} from '../../../../services/api/aiSettingsApi';
import { openhumanUpdateLocalAiSettings } from '../../../../utils/tauriCommands/config';
import { presentProviderSetupError } from '../ProviderSetupErrorNotice';
import {
  type AISettings,
  authStyleForSlug,
  BUILTIN_PROVIDER_META,
  type CloudProvider,
  defaultEndpointFor,
  maskKeyLabel,
} from './aiPanelTypes';

export type ConnectCredentialMode =
  | 'api_key'
  | 'oauth'
  | 'codex_oauth'
  | 'endpoint'
  | 'endpoint_key'
  | 'cli_login';

export function useProviderConnect({
  draft,
  saved,
  persist,
  t,
  onConnected,
}: {
  draft: AISettings;
  saved: AISettings;
  persist: (next: AISettings) => Promise<void>;
  t: (key: string, fallback?: string) => string;
  /** Called once a credential is successfully saved — clears whichever
   *  dialog-open state the caller is tracking. */
  onConnected: () => void;
}) {
  const [busyAction, setBusyAction] = useState<string | null>(null);
  const [codexAuthError, setCodexAuthError] = useState<string | null>(null);
  const [providerAuthErrors, setProviderAuthErrors] = useState<ProviderAuthError[]>([]);
  // #5339: non-fatal "the key was saved, but the provider was unreachable"
  // advisory. Keyed by slug (#5341) so it is cleared only for the provider it
  // belongs to.
  const [providerSaveNotice, setProviderSaveNotice] = useState<{
    slug: string;
    message: string;
  } | null>(null);

  useEffect(() => {
    let cancelled = false;
    void loadProviderAuthErrors()
      .then(errs => {
        if (!cancelled) setProviderAuthErrors(errs);
      })
      .catch(() => {
        // Best-effort surface — a fetch failure must not break the panel.
        if (!cancelled) setProviderAuthErrors([]);
      });
    return () => {
      cancelled = true;
    };
  }, [saved]);

  const connectProvider = useCallback(
    async ({
      slug,
      localLabel = null,
      value,
      endpoint: endpointOverride,
      credentialMode,
    }: {
      slug: string;
      localLabel?: string | null;
      value: string;
      endpoint?: string | null;
      credentialMode: ConnectCredentialMode;
    }) => {
      const isLocalRuntime = credentialMode === 'endpoint' || credentialMode === 'endpoint_key';
      const isEndpointKey = credentialMode === 'endpoint_key';
      const isCodexOAuth = credentialMode === 'codex_oauth';
      const isCliLogin = credentialMode === 'cli_login';
      setBusyAction(`toggle-${localLabel ? localLabel.toLowerCase().replace(/\s/g, '') : slug}`);
      // A fresh attempt on THIS provider clears only its own prior advisory —
      // an advisory about a different provider must survive (#5341).
      setProviderSaveNotice(prev => (prev?.slug === slug ? null : prev));

      try {
        const trimmed = value.trim();
        const rawEndpoint = isEndpointKey ? (endpointOverride ?? '').trim() : trimmed;
        const endpoint = isLocalRuntime
          ? (() => {
              const url = new URL(rawEndpoint);
              if (!/^https?:$/.test(url.protocol)) {
                throw new Error('Endpoint must start with http:// or https://');
              }
              if (url.pathname === '' || url.pathname === '/') {
                url.pathname = '/v1';
              }
              return url.toString().replace(/\/$/, '');
            })()
          : defaultEndpointFor(slug);

        const upserted: CloudProvider = {
          id: `p_${slug}_${Math.random().toString(36).slice(2, 7)}`,
          slug,
          label: localLabel ?? BUILTIN_PROVIDER_META[slug]?.label ?? slug,
          endpoint,
          authStyle: authStyleForSlug(slug),
          // CLI-login providers hold no API key — reflect that honestly.
          maskedKey: maskKeyLabel(!isCliLogin),
        };

        const priorWireProviders = saved.cloudProviders.map(p => ({
          id: p.id,
          slug: p.slug,
          label: p.label,
          endpoint: p.endpoint,
          auth_style: p.authStyle,
        }));

        if (!isLocalRuntime && !isCodexOAuth && !isCliLogin && slug !== 'openhuman') {
          await setCloudProviderKey(slug, trimmed);
        } else if (isLocalRuntime && slug === 'ollama') {
          const baseUrl = endpoint.replace(/\/v1\/?$/, '');
          await openhumanUpdateLocalAiSettings({
            base_url: baseUrl,
            provider: 'ollama',
            runtime_enabled: true,
            opt_in_confirmed: true,
          });
        } else if (isLocalRuntime && slug === 'lmstudio') {
          await openhumanUpdateLocalAiSettings({
            base_url: endpoint,
            provider: 'lm_studio',
            runtime_enabled: true,
            opt_in_confirmed: true,
          });
        } else if (isLocalRuntime && slug === 'omlx') {
          // OMLX: OpenAI-compatible local runtime that also requires a Bearer
          // key. Persist both the endpoint and the key into local_ai (the Rust
          // factory's omlx branch reads `local_ai.api_key` as the Bearer token).
          await openhumanUpdateLocalAiSettings({
            base_url: endpoint,
            api_key: trimmed,
            provider: 'omlx',
            runtime_enabled: true,
            opt_in_confirmed: true,
          });
        }

        if (slug !== 'openhuman') {
          const nextWireProviders = [
            ...priorWireProviders.filter(p => p.slug !== slug),
            {
              id: upserted.id,
              slug: upserted.slug,
              label: upserted.label,
              endpoint: upserted.endpoint,
              auth_style: upserted.authStyle,
            },
          ];
          await flushCloudProviders(nextWireProviders);
          if (!isCodexOAuth && !isCliLogin) {
            try {
              await listProviderModels(slug);
            } catch (probeErr) {
              const msg = probeErr instanceof Error ? probeErr.message : String(probeErr);
              const reason = classifyProviderVerificationFailure(msg);
              const isKeyProvider = !isLocalRuntime && slug !== 'openhuman';
              if (isKeyProvider && reason !== 'auth') {
                // #5339: transient / unreachable / unknown — the key is
                // plausibly valid, so keep it and record a non-fatal advisory.
                console.warn(
                  `[ai-settings] provider=${slug} add-time probe non-fatal reason=${reason}`
                );
                setProviderSaveNotice({
                  slug,
                  message: describeProviderVerificationFailure(slug, msg, t),
                });
              } else {
                // Auth failure (wrong key), or a local runtime that isn't up:
                // roll both stores back and reject so the user fixes it.
                await flushCloudProviders(priorWireProviders).catch(rollbackErr =>
                  console.warn(`[ai-settings] rollback flush failed slug=${slug}`, rollbackErr)
                );
                if (isKeyProvider) {
                  await clearCloudProviderKey(slug).catch(rollbackErr =>
                    console.warn(
                      `[ai-settings] rollback clearCloudProviderKey failed slug=${slug}`,
                      rollbackErr
                    )
                  );
                }
                throw new Error(`Could not reach ${upserted.label}: ${msg}`);
              }
            }
          }
        }

        const nextDraft = {
          ...draft,
          cloudProviders: [...draft.cloudProviders.filter(p => p.slug !== slug), upserted],
        };
        await persist(nextDraft);
        if (isCodexOAuth && slug === 'openai') {
          await clearCloudProviderKey(slug);
        }
        if (slug === 'openai') {
          setCodexAuthError(null);
        }
        onConnected();
      } finally {
        setBusyAction(null);
      }
    },
    [draft, persist, saved.cloudProviders, t, onConnected]
  );

  const connectOpenAiViaCodexAuth = useCallback(async () => {
    setCodexAuthError(null);
    setBusyAction('codex-auth');
    try {
      await importOpenAiCodexCliAuth();
      await connectProvider({ slug: 'openai', value: 'oauth', credentialMode: 'codex_oauth' });
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      const localizedMessage =
        message === OPENAI_CODEX_OAUTH_MISSING_AUTH_URL
          ? t('settings.ai.codexOauthMissingAuthUrl')
          : message === OPENAI_CODEX_OAUTH_MISSING_CALLBACK_URL
            ? t('settings.ai.codexOauthMissingCallbackUrl')
            : message;
      console.warn('[ai-settings] codex auth import failed', {
        summary: presentProviderSetupError(message, t).summary,
      });
      setCodexAuthError(localizedMessage);
    } finally {
      setBusyAction(null);
    }
  }, [connectProvider, t]);

  return {
    busyAction,
    setBusyAction,
    codexAuthError,
    providerAuthErrors,
    providerSaveNotice,
    setProviderSaveNotice,
    connectProvider,
    connectOpenAiViaCodexAuth,
  };
}
