/*
 * Cloud provider editor modal — the advanced "add custom provider" / "edit
 * provider" flow (name, OpenAI-compatible URL, API key).
 */
import { useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import Button from '../../../ui/Button';
import Label from '../../../ui/Label';
import { ModalShell } from '../../../ui/ModalShell';
import TextField from '../../../ui/TextField';
import { isAzureFoundryEndpoint, isAzureV1BaseUrl } from '../azureDeployment';
import { presentProviderSetupError, ProviderSetupErrorNotice } from '../ProviderSetupErrorNotice';
import {
  BUILTIN_RESERVED_SLUGS,
  type CloudProvider,
  formatI18n,
  maskKeyLabel,
  ProviderProbeError,
  slugifyCustomProviderName,
} from './aiPanelTypes';

export const CloudProviderEditor = ({
  initial,
  existingSlugs,
  onClose,
  onSubmit,
  onClearKey,
}: {
  initial: CloudProvider | null;
  existingSlugs: string[];
  onClose: () => void;
  onSubmit: (
    next: CloudProvider,
    apiKey: string,
    opts?: { skipProbe?: boolean }
  ) => Promise<void> | void;
  onClearKey: (slug: string) => Promise<void> | void;
}) => {
  const { t } = useT();
  const [label, setLabel] = useState<string>(initial?.label ?? '');
  const [endpoint, setEndpoint] = useState(initial?.endpoint ?? '');
  const [apiKey, setApiKey] = useState('');
  const [saving, setSaving] = useState(false);
  const [submitError, setSubmitError] = useState<string | null>(null);
  // Set once the live `/models` verification has rejected, which unlocks the
  // "add without verifying" path. Only a probe failure earns it — a bad slug or
  // a failed key write must still block (#5213).
  const [probeFailed, setProbeFailed] = useState(false);
  const slug = initial?.slug ?? slugifyCustomProviderName(label);
  const hasReservedSlugCollision = !initial && BUILTIN_RESERVED_SLUGS.includes(slug);
  const slugError = !slug
    ? t('settings.ai.slugMissingError')
    : existingSlugs.includes(slug)
      ? t('settings.ai.slugInUseError')
      : hasReservedSlugCollision
        ? t('settings.ai.slugReservedError')
        : null;
  const hasExistingKey = (initial?.maskedKey ?? '').startsWith('••••');
  // Skipping verification is a bet that the provider works despite an
  // unreadable listing. For an Azure host that is not the `/openai/v1` base
  // that bet is already lost: `{base}/chat/completions` is not a route Azure
  // serves there and the stored bearer auth is the wrong header, so the entry
  // would be dead on arrival. Withhold the bypass and let the inline nudge do
  // its job instead of manufacturing a broken provider (#5213).
  const knownUnusableEndpoint =
    isAzureFoundryEndpoint(endpoint) && !isAzureV1BaseUrl(endpoint.trim());

  const submitProvider = async (opts?: { skipProbe?: boolean }) => {
    setSaving(true);
    setSubmitError(null);
    // Cleared alongside the error: a later attempt that fails for an unrelated
    // reason (slug collision, key write) must not still offer to skip
    // verification, which is the distinction `ProviderProbeError` exists for.
    setProbeFailed(false);
    try {
      if (slugError) {
        throw new Error(slugError);
      }
      await onSubmit(
        {
          id: initial?.id ?? '',
          slug,
          label: label.trim() || slug,
          endpoint: endpoint.trim(),
          authStyle: initial?.authStyle ?? 'bearer',
          maskedKey: maskKeyLabel(hasExistingKey || apiKey.length > 0),
        },
        apiKey.trim(),
        opts
      );
    } catch (err) {
      // Surface the failure inline and keep the dialog open so the user can fix
      // the key/URL and retry. A rejected `/models` probe additionally unlocks
      // the "add without verifying" button — the listing is a convenience for
      // the model dropdown, not a precondition for inference.
      const message = err instanceof Error ? err.message : String(err);
      console.warn('[ai-settings] cloud provider editor submit failed', {
        slug,
        probeFailure: err instanceof ProviderProbeError,
        summary: presentProviderSetupError(message, t).summary,
      });
      setSubmitError(message);
      if (err instanceof ProviderProbeError) {
        setProbeFailed(true);
      }
    } finally {
      setSaving(false);
    }
  };

  return (
    <ModalShell
      titleId="cloud-provider-form-title"
      title={
        initial
          ? formatI18n(t('settings.ai.editProvider'), { label: initial.label })
          : t('settings.ai.addCloudProvider')
      }
      subtitle={
        <>
          {t('settings.ai.apiKeysEncrypted')} <span className="font-mono">auth-profiles.json</span>.
        </>
      }
      onClose={onClose}
      // The hand-rolled dialog had no backdrop, escape or close affordance at
      // all: Cancel was the only way out and it was `disabled` while saving.
      // Radix supplies all three, so re-apply that guard or an in-flight save
      // can be dismissed out from under itself.
      closePolicy={saving ? { escape: false, backdrop: false, button: false } : undefined}
      contentClassName="space-y-3 px-4 py-3"
      footer={
        <div className="flex items-center justify-end gap-2">
          <Button variant="secondary" size="xs" onClick={onClose} disabled={saving}>
            {t('common.cancel')}
          </Button>
          {probeFailed && !knownUnusableEndpoint ? (
            <Button
              variant="secondary"
              size="xs"
              analyticsId="ai-provider-add-without-verifying"
              disabled={saving || !endpoint.trim() || Boolean(slugError)}
              onClick={() => void submitProvider({ skipProbe: true })}>
              {t('settings.ai.probeFailedAddAnyway')}
            </Button>
          ) : null}
          <Button
            variant="primary"
            size="xs"
            disabled={saving || !endpoint.trim() || Boolean(slugError)}
            onClick={() => void submitProvider()}>
            {saving
              ? t('settings.ai.saving')
              : initial
                ? t('settings.ai.saveChanges')
                : t('settings.ai.addProvider')}
          </Button>
        </div>
      }>
      <div>
        <Label htmlFor="cloud-provider-name" className="text-xs text-content-secondary">
          {t('common.name')}
        </Label>
        <TextField
          id="cloud-provider-name"
          value={label}
          onChange={e => setLabel(e.target.value)}
          className="mt-1"
          placeholder={t('settings.ai.providerNamePlaceholder')}
        />
        <div className="mt-1 text-[11px] text-content-muted">
          {t('settings.ai.slugLabel')}{' '}
          <span className="font-mono text-content-secondary">
            {slug || t('settings.ai.noneDash')}
          </span>
        </div>
        {slugError ? (
          <div className="mt-1 text-[11px] text-coral-600 dark:text-coral-300">{slugError}</div>
        ) : null}
      </div>
      <div>
        <Label htmlFor="cloud-provider-openai-url" className="text-xs text-content-secondary">
          {t('settings.ai.openAiUrlLabel')}
        </Label>
        <TextField
          id="cloud-provider-openai-url"
          mono
          value={endpoint}
          onChange={e => setEndpoint(e.target.value)}
          className="mt-1"
          placeholder={t('settings.ai.openAiUrlPlaceholder')}
        />
        {/* Azure routes by deployment name, which is set on the model
                field rather than here — point the user at it (#5213). */}
        {isAzureFoundryEndpoint(endpoint) && (
          <div className="mt-1 text-[11px] text-content-muted">
            {t('settings.ai.deploymentNameProviderHint')}
          </div>
        )}
        {/* Only Azure's `/openai/v1` base is OpenAI-shaped: it serves a
                `/models` listing and accepts the resource key as a bearer
                token, which is the auth style every custom provider is stored
                with. The older `api-version` surface wants an `api-key` header
                and no `/models`, so a user who pastes the portal's bare
                resource URL fails both the probe and inference (#5213). */}
        {knownUnusableEndpoint && (
          <div className="mt-1 text-[11px] text-amber-700 dark:text-amber-300">
            {t('settings.ai.azureV1EndpointHint')}
          </div>
        )}
      </div>
      <div>
        <div className="flex items-center justify-between gap-2">
          <Label htmlFor="cloud-provider-api-key" className="text-xs text-content-secondary">
            {t('settings.ai.apiKeyFieldLabel')}
          </Label>
          {hasExistingKey && (
            <Button
              variant="tertiary"
              tone="danger"
              size="xs"
              onClick={() => void onClearKey(slug)}>
              {t('settings.ai.clearStoredKey')}
            </Button>
          )}
        </div>
        <TextField
          id="cloud-provider-api-key"
          aria-label={t('settings.ai.apiKeyFieldLabel')}
          type="text"
          mono
          autoComplete="off"
          autoCorrect="off"
          autoCapitalize="off"
          spellCheck={false}
          data-form-type="other"
          data-lpignore="true"
          data-1p-ignore="true"
          value={apiKey}
          onChange={e => setApiKey(e.target.value)}
          className="mt-1"
          placeholder={hasExistingKey ? t('settings.ai.keepExistingKeyPlaceholder') : 'sk-...'}
        />
      </div>
      {submitError ? <ProviderSetupErrorNotice error={submitError} /> : null}
      {/* A failed verification is not a failed provider. Explain what the
              probe does and does not prove, then let the user proceed (#5213).
              Withheld for an endpoint we already know cannot serve inference. */}
      {probeFailed && !knownUnusableEndpoint ? (
        <p className="rounded-lg border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 px-3 py-2 text-[11px] text-amber-800 dark:text-amber-200">
          {t('settings.ai.probeFailedHint')}
        </p>
      ) : null}
    </ModalShell>
  );
};

export default CloudProviderEditor;
