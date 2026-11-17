/*
 * Provider-connect orchestration: writing a credential (API key / endpoint /
 * OAuth / CLI login), live-probing it, and rolling back on failure. Shared by
 * the built-in cloud provider chips, the local-runtime chips, and the Codex
 * connect button.
 */
import { useCallback, useEffect, useRef, useState } from 'react';

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

const normalizeProviderSlug = (slug: string) => slug.trim().toLowerCase();
const isProviderSlug = (candidate: string, slug: string) =>
  normalizeProviderSlug(candidate) === normalizeProviderSlug(slug);

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
  const latestSettings = useRef({ draft, saved });
  const providerConnectQueue = useRef(Promise.resolve());
  const providerConnectRevisions = useRef(new Map<string, number>());
  useEffect(() => {
    latestSettings.current = { draft, saved };
  }, [draft, saved]);

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
      slug = normalizeProviderSlug(slug);
      const revision = (providerConnectRevisions.current.get(slug) ?? 0) + 1;
      providerConnectRevisions.current.set(slug, revision);
      const previousOperation = providerConnectQueue.current;
      let finishOperation!: () => void;
      const currentOperation = new Promise<void>(resolve => {
        finishOperation = resolve;
      });
      providerConnectQueue.current = previousOperation.then(() => currentOperation);
      if (previousOperation) await previousOperation;
      if (revision !== providerConnectRevisions.current.get(slug)) {
        finishOperation();
        return;
      }

      const isLocalRuntime = credentialMode === 'endpoint' || credentialMode === 'endpoint_key';
      const isEndpointKey = credentialMode === 'endpoint_key';
      const isCodexOAuth = credentialMode === 'codex_oauth';
      const isCliLogin = credentialMode === 'cli_login';
      setBusyAction(`toggle-${localLabel ? localLabel.toLowerCase().replace(/\s/g, '') : slug}`);
      // A fresh attempt on THIS provider clears only its own prior advisory —
      // an advisory about a different provider must survive (#5341).
      setProviderSaveNotice(prev => (prev && isProviderSlug(prev.slug, slug) ? null : prev));

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

        const initialSettings = latestSettings.current;
        const initialProvider = initialSettings.draft.cloudProviders.find(provider =>
          isProviderSlug(provider.slug, slug)
        );
        const initialSavedProvider = initialSettings.saved.cloudProviders.find(provider =>
          isProviderSlug(provider.slug, slug)
        );
        const upserted: CloudProvider = {
          id: `p_${slug}_${Math.random().toString(36).slice(2, 7)}`,
          slug,
          label: localLabel ?? BUILTIN_PROVIDER_META[slug]?.label ?? slug,
          endpoint,
          caCertPem: initialProvider?.caCertPem ?? initialSavedProvider?.caCertPem,
          authStyle: authStyleForSlug(slug),
          // CLI-login providers hold no API key — reflect that honestly.
          maskedKey: maskKeyLabel(!isCliLogin),
        };

        if (isLocalRuntime && slug === 'ollama') {
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
          const currentSettings = latestSettings.current;
          const currentProvider = currentSettings.draft.cloudProviders.find(provider =>
            isProviderSlug(provider.slug, slug)
          );
          const currentSavedProvider = currentSettings.saved.cloudProviders.find(provider =>
            isProviderSlug(provider.slug, slug)
          );
          const currentUpserted = {
            ...upserted,
            caCertPem: currentProvider?.caCertPem ?? currentSavedProvider?.caCertPem,
          };
          const priorWireProviders = currentSettings.saved.cloudProviders.map(provider => ({
            id: provider.id,
            slug: normalizeProviderSlug(provider.slug),
            label: provider.label,
            endpoint: provider.endpoint,
            ca_cert_pem: provider.caCertPem ?? '',
            auth_style: provider.authStyle,
          }));
          const nextWireProviders = [
            ...priorWireProviders.filter(provider => !isProviderSlug(provider.slug, slug)),
            {
              id: currentUpserted.id,
              slug: currentUpserted.slug,
              label: currentUpserted.label,
              endpoint: currentUpserted.endpoint,
              ca_cert_pem: currentUpserted.caCertPem ?? '',
              auth_style: currentUpserted.authStyle,
            },
          ];
          let flushedProviders = nextWireProviders;
          try {
            await flushCloudProviders(flushedProviders);
            if (!isLocalRuntime && !isCodexOAuth && !isCliLogin) {
              await setCloudProviderKey(slug, trimmed);
            }
            const latestSettingsAfterKey = latestSettings.current;
            const latestDraftProvider = latestSettingsAfterKey.draft.cloudProviders.find(provider =>
              isProviderSlug(provider.slug, slug)
            );
            const latestSavedProvider = latestSettingsAfterKey.saved.cloudProviders.find(provider =>
              isProviderSlug(provider.slug, slug)
            );
            const latestCaCertPem =
              latestDraftProvider?.caCertPem ?? latestSavedProvider?.caCertPem ?? '';
            if (latestCaCertPem !== (currentUpserted.caCertPem ?? '')) {
              flushedProviders = flushedProviders.map(provider =>
                isProviderSlug(provider.slug, slug)
                  ? { ...provider, ca_cert_pem: latestCaCertPem }
                  : provider
              );
              await flushCloudProviders(flushedProviders);
            }
          } catch (writeError) {
            await flushCloudProviders(priorWireProviders).catch(rollbackErr =>
              console.warn(
                `[ai-settings] rollback flush after provider write failure slug=${slug}`,
                rollbackErr
              )
            );
            throw writeError;
          }
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

        const currentSettings = latestSettings.current;
        const currentProvider = currentSettings.draft.cloudProviders.find(provider =>
          isProviderSlug(provider.slug, slug)
        );
        const currentSavedProvider = currentSettings.saved.cloudProviders.find(provider =>
          isProviderSlug(provider.slug, slug)
        );
        const finalUpserted = {
          ...upserted,
          caCertPem: currentProvider?.caCertPem ?? currentSavedProvider?.caCertPem,
        };
        if (revision !== providerConnectRevisions.current.get(slug)) return;
        const nextDraft = {
          ...currentSettings.draft,
          cloudProviders: [
            ...currentSettings.draft.cloudProviders.filter(
              provider => !isProviderSlug(provider.slug, slug)
            ),
            finalUpserted,
          ],
        };
        await persist(nextDraft);
        // `persist` updates React state asynchronously. The next queued
        // provider submission must still build its replacement list from
        // this successfully published snapshot, even before React rerenders.
        latestSettings.current = { draft: nextDraft, saved: nextDraft };
        if (revision !== providerConnectRevisions.current.get(slug)) return;
        if (isCodexOAuth && slug === 'openai') {
          await clearCloudProviderKey(slug);
        }
        if (slug === 'openai') {
          setCodexAuthError(null);
        }
        onConnected();
      } finally {
        setBusyAction(null);
        finishOperation();
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
