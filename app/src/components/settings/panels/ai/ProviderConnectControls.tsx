/*
 * Provider connect controls — the toggle chip that turns a provider on/off,
 * and the dialog it opens to collect credentials.
 */
import { Dialog as DialogPrimitive } from 'radix-ui';
import { useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import { openUrl } from '../../../../utils/openUrl';
import Button from '../../../ui/Button';
import { DialogContent, DialogRoot } from '../../../ui/Dialog';
import Label from '../../../ui/Label';
import TextField from '../../../ui/TextField';
import OpenAiOAuthConnect from '../../oauth/OpenAiOAuthConnect';
import { builtinCloudProvider } from '../builtinCloudProviders';
import { presentProviderSetupError, ProviderSetupErrorNotice } from '../ProviderSetupErrorNotice';
import { defaultEndpointFor, formatI18n, KIMI_PLATFORM_URL } from './aiPanelTypes';

// Connect-provider dialog — shown when the user flips a provider toggle ON.
//
// Two modes:
//   - apiKey: cloud providers (OpenAI, Anthropic, …). Collects a secret.
//   - endpoint: local runtimes (Ollama, LM Studio). Collects an HTTP URL
//     (and optionally an API key for OpenAI-compatible self-hosted setups).
//
// The parent decides how to persist: cloud → auth-profiles, local → both
// the cloud_providers entry's `endpoint` (so /models discovery works) and
// `local_ai.base_url` (so the Rust factory's Ollama branch routes to it).
export const ProviderKeyDialog = ({
  slug,
  label,
  isLocalRuntime,
  endpointKeyMode = false,
  initialValue,
  initialKeyValue,
  oauthAction,
  openAiOAuth,
  onCancel,
  onSubmit,
}: {
  slug: string;
  label: string;
  /** When true, render an "Endpoint URL" field instead of API key. */
  isLocalRuntime: boolean;
  /**
   * When true (OMLX), render BOTH an "Endpoint URL" field AND an "API key"
   * field. `onSubmit` then receives the API key as `value` and the endpoint
   * via the `endpoint` argument.
   */
  endpointKeyMode?: boolean;
  /** Pre-populate the field when editing an existing provider's endpoint. */
  initialValue?: string;
  /** Pre-populate the API key field in `endpointKeyMode`. */
  initialKeyValue?: string;
  oauthAction?: { label: string; description?: string; onClick: () => Promise<void> | void } | null;
  /** Register or remove the provider after the core OAuth operation succeeds. */
  openAiOAuth?: {
    onCompleted: () => Promise<void> | void;
    onDisconnected: () => Promise<void> | void;
  } | null;
  onCancel: () => void;
  /** Returns the entered value(s). For plain local runtimes this is the
   *  endpoint URL; for cloud providers it's the API key. In `endpointKeyMode`
   *  the API key is `value` and the endpoint URL is `endpoint`. */
  onSubmit: (value: string, endpoint?: string) => Promise<void> | void;
}) => {
  const { t } = useT();
  // In `endpointKeyMode`, `value` holds the endpoint URL and `keyValue` holds
  // the API key. Otherwise `value` is either the endpoint (local) or key (cloud).
  const [value, setValue] = useState<string>(
    initialValue ?? (isLocalRuntime ? defaultEndpointFor(slug) : '')
  );
  const [keyValue, setKeyValue] = useState<string>(initialKeyValue ?? '');
  const [phase, setPhase] = useState<'idle' | 'saving' | 'oauth'>('idle');
  const [error, setError] = useState<string | null>(null);
  const busy = phase !== 'idle';

  const placeholder = isLocalRuntime
    ? defaultEndpointFor(slug) || t('settings.ai.defaultLocalEndpoint')
    : (builtinCloudProvider(slug)?.keyPlaceholder ?? 'your-api-key');
  const keyPlaceholder = builtinCloudProvider(slug)?.keyPlaceholder ?? 'your-api-key';

  const fieldLabel = isLocalRuntime
    ? t('settings.ai.endpointUrlLabel')
    : t('settings.ai.apiKeyFieldLabel');
  const helper = isLocalRuntime
    ? formatI18n(t('settings.ai.localRuntimeHelper'), { label })
    : t('settings.ai.apiKeyStoredEncrypted');
  const platformLinkUrl = slug === 'moonshot' && !isLocalRuntime ? KIMI_PLATFORM_URL : null;
  const titleId = 'provider-key-dialog-title';

  const handleSave = async () => {
    const trimmed = value.trim();
    const trimmedKey = keyValue.trim();
    if (!trimmed) {
      setError(
        isLocalRuntime ? t('settings.ai.endpointUrlRequired') : t('settings.ai.apiKeyRequired')
      );
      return;
    }
    if (isLocalRuntime && !/^https?:\/\//i.test(trimmed)) {
      setError(t('settings.ai.endpointProtocolRequired'));
      return;
    }
    if (endpointKeyMode && !trimmedKey) {
      setError(t('settings.ai.apiKeyRequired'));
      return;
    }
    setError(null);

    // A provider credential is being saved. This adds/updates a `cloudProviders`
    // entry only — it does NOT change the workload routing map, so routing is
    // unchanged afterwards (see inferRoutingMode). Logged for routing diagnostics.
    console.debug('[ai-settings][routing] saving provider credential', {
      slug,
      local_runtime: isLocalRuntime,
      kind: endpointKeyMode ? 'endpointKey' : isLocalRuntime ? 'endpoint' : 'apiKey',
    });

    setPhase('saving');
    try {
      // In endpointKeyMode the API key is the primary value, endpoint is the
      // second arg; otherwise the single field is the primary value.
      if (endpointKeyMode) {
        await onSubmit(trimmedKey, trimmed);
      } else {
        await onSubmit(trimmed);
      }
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.warn('[ai-settings] provider setup failed', {
        slug,
        local_runtime: isLocalRuntime,
        summary: presentProviderSetupError(message, t).summary,
      });
      setError(message);
      setPhase('idle');
    }
  };

  const handleOAuth = async () => {
    if (!oauthAction) return;
    setError(null);
    setPhase('oauth');
    try {
      await oauthAction.onClick();
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.warn('[ai-settings] provider oauth failed', {
        slug,
        summary: presentProviderSetupError(message, t).summary,
      });
      setError(message);
      setPhase('idle');
    }
  };

  // This dialog does NOT use `ModalShell`: the Kimi/Moonshot "Get API key"
  // link floats in the header's top-right corner (a plain `<a>` positioned
  // absolutely, logical-inline-aware for RTL) rather than living in a shared
  // subtitle slot, and the heading text itself is `t('connectProvider')`
  // (e.g. "Connect") + label, distinct from the dialog's own accessible name
  // (`connectProviderDialog`, e.g. "Connect {label}") — two different i18n
  // strings that read identically in English but diverge in translation.
  // `ModalShell`'s single `title` slot can only serve one of the two, so this
  // composes `DialogRoot`/`DialogContent` directly, which still gets the real
  // focus trap, scroll lock,
  // and `aria-hidden`-the-rest-of-the-tree behavior `ModalShell` is built on.
  return (
    <DialogRoot
      open
      onOpenChange={next => {
        if (!next && !busy) onCancel();
      }}>
      <DialogContent
        aria-labelledby={titleId}
        onEscapeKeyDown={event => {
          if (busy) event.preventDefault();
        }}
        onPointerDownOutside={event => {
          if (busy) event.preventDefault();
        }}
        onInteractOutside={event => {
          if (busy) event.preventDefault();
        }}
        className="border border-line p-6 shadow-soft">
        {platformLinkUrl ? (
          <a
            href={platformLinkUrl}
            target="_blank"
            rel="noopener noreferrer"
            onClick={event => {
              event.preventDefault();
              void openUrl(platformLinkUrl).catch(err => {
                console.warn('[ai-settings] provider platform link open failed', {
                  slug,
                  error: err instanceof Error ? err.message : String(err),
                });
              });
            }}
            style={{ insetInlineEnd: '1.5rem' }}
            className="absolute top-6 text-xs font-medium leading-6 text-primary-600 hover:text-primary-700 dark:text-primary-300 dark:hover:text-primary-200">
            {t('settings.ai.getProviderApiKey')}
          </a>
        ) : null}
        <div className="mb-4" style={platformLinkUrl ? { paddingInlineEnd: '9rem' } : undefined}>
          <DialogPrimitive.Title asChild>
            <h3 id={titleId} className="text-base font-semibold text-content">
              {`${t('settings.ai.connectProvider')} ${label}`}
            </h3>
          </DialogPrimitive.Title>
          <p className="mt-0.5 text-xs text-content-muted">{helper}</p>
        </div>

        <div className="flex flex-col gap-1.5">
          <Label htmlFor="provider-key-input" className="text-xs text-content-secondary">
            {fieldLabel}
          </Label>
          <TextField
            id="provider-key-input"
            type={isLocalRuntime ? 'url' : 'text'}
            mono={isLocalRuntime}
            autoComplete="off"
            autoCorrect="off"
            autoCapitalize="off"
            spellCheck={false}
            data-form-type="other"
            data-lpignore="true"
            data-1p-ignore="true"
            value={value}
            placeholder={placeholder}
            disabled={busy}
            onChange={e => {
              setValue(e.target.value);
              setError(null);
            }}
          />
          {/* OMLX (endpointKeyMode): render the API key field in addition to
              the endpoint field above — the runtime is OpenAI-compatible but
              gated behind a Bearer key. */}
          {endpointKeyMode ? (
            <>
              <Label
                htmlFor="provider-key-input-key"
                className="mt-3 text-xs text-content-secondary">
                {t('settings.ai.apiKeyFieldLabel')}
              </Label>
              <TextField
                id="provider-key-input-key"
                type="text"
                autoComplete="off"
                autoCorrect="off"
                autoCapitalize="off"
                spellCheck={false}
                data-form-type="other"
                data-lpignore="true"
                data-1p-ignore="true"
                value={keyValue}
                placeholder={keyPlaceholder}
                disabled={busy}
                onChange={e => {
                  setKeyValue(e.target.value);
                  setError(null);
                }}
              />
            </>
          ) : null}
          {error ? <ProviderSetupErrorNotice error={error} /> : null}
        </div>

        {openAiOAuth ? (
          <div className="mt-4">
            <OpenAiOAuthConnect
              testIdPrefix="settings-openai-oauth"
              allowDisconnect
              onCompleted={openAiOAuth.onCompleted}
              onDisconnected={openAiOAuth.onDisconnected}
            />
          </div>
        ) : null}

        {oauthAction ? (
          <div className="mt-4 rounded-xl border border-line bg-surface-muted dark:bg-surface-muted/50 p-3">
            <div className="text-[11px] font-semibold uppercase tracking-wide text-content-muted">
              {t('settings.ai.or')}
            </div>
            <p className="mt-1 text-xs text-content-muted">
              {oauthAction.description ?? t('settings.ai.openRouterOauthDescription')}
            </p>
            <Button
              type="button"
              variant="secondary"
              size="sm"
              onClick={() => void handleOAuth()}
              disabled={busy}
              className="mt-3">
              {phase === 'oauth' ? t('settings.ai.connecting') : oauthAction.label}
            </Button>
          </div>
        ) : null}
        <div className="mt-6 flex justify-end gap-2">
          <Button type="button" variant="secondary" size="sm" onClick={onCancel} disabled={busy}>
            {t('common.cancel')}
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            onClick={() => void handleSave()}
            disabled={busy}>
            {phase === 'saving' ? t('settings.ai.saving') : t('common.save')}
          </Button>
        </div>
      </DialogContent>
    </DialogRoot>
  );
};
