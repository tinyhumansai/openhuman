/*
 * OCI Generative AI settings dialog.
 *
 * Collects the non-secret settings kept on the provider entry (auth mode,
 * profile, region, compartment, project, protocol, dedicated endpoint) plus —
 * only in `api_key` mode — the Generative AI API key, which the caller writes
 * to the keyring like any other provider key. Profile and principal modes
 * store no secret: key material stays in the user's OCI config directory (or
 * the runtime environment) and is read by the core when a model is built.
 */
import { Dialog as DialogPrimitive } from 'radix-ui';
import { useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import type { OciAuthMode, OciSettings } from '../../../../utils/tauriCommands/config';
import Button from '../../../ui/Button';
import { DialogContent, DialogRoot } from '../../../ui/Dialog';
import Label from '../../../ui/Label';
import NativeSelect from '../../../ui/NativeSelect';
import TextField from '../../../ui/TextField';
import { presentProviderSetupError, ProviderSetupErrorNotice } from '../ProviderSetupErrorNotice';
import {
  DEFAULT_OCI_SETTINGS,
  isOciGatedPrincipalMode,
  isOciPrincipalMode,
  isOciProfileMode,
  normalizeOciSettings,
  OCI_AUTH_MODE_OPTIONS,
  OCI_PROVIDER_SLUG,
  OCI_TRANSPORT_OPTIONS,
  ociTransportUsesProject,
  validateOciSettings,
} from './ociSettings';

const FIELD_PROPS = {
  autoComplete: 'off',
  autoCorrect: 'off',
  autoCapitalize: 'off',
  spellCheck: false,
  'data-form-type': 'other',
  'data-lpignore': 'true',
  'data-1p-ignore': 'true',
} as const;

export const OciSettingsDialog = ({
  label,
  initial,
  hasStoredKey,
  onCancel,
  onSubmit,
}: {
  label: string;
  /** Existing settings when editing a connected OCI provider. */
  initial?: OciSettings;
  /** True when a Generative AI API key is already stored for this provider. */
  hasStoredKey: boolean;
  onCancel: () => void;
  /** Receives the normalised settings and the (possibly empty) API key. */
  onSubmit: (settings: OciSettings, apiKey: string) => Promise<void> | void;
}) => {
  const { t } = useT();
  const [settings, setSettings] = useState<OciSettings>(initial ?? DEFAULT_OCI_SETTINGS);
  const [apiKey, setApiKey] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const titleId = 'oci-settings-dialog-title';
  const mode = settings.auth_mode;

  const patch = (next: Partial<OciSettings>) => {
    setSettings(prev => ({ ...prev, ...next }));
    setError(null);
  };

  const handleSave = async () => {
    const problem = validateOciSettings(settings, apiKey, hasStoredKey);
    if (problem) {
      setError(t(problem));
      return;
    }
    setError(null);
    setSaving(true);
    console.debug('[ai-settings][oci] saving provider settings', {
      auth_mode: mode,
      transport: settings.transport,
    });
    try {
      await onSubmit(normalizeOciSettings(settings), mode === 'api_key' ? apiKey.trim() : '');
    } catch (err) {
      const message = err instanceof Error ? err.message : String(err);
      console.warn('[ai-settings][oci] provider setup failed', {
        slug: OCI_PROVIDER_SLUG,
        summary: presentProviderSetupError(message, t).summary,
      });
      setError(message);
      setSaving(false);
    }
  };

  return (
    <DialogRoot
      open
      onOpenChange={next => {
        if (!next && !saving) onCancel();
      }}>
      <DialogContent
        aria-labelledby={titleId}
        onEscapeKeyDown={event => {
          if (saving) event.preventDefault();
        }}
        onPointerDownOutside={event => {
          if (saving) event.preventDefault();
        }}
        onInteractOutside={event => {
          if (saving) event.preventDefault();
        }}
        className="border border-line p-6 shadow-soft">
        <div className="mb-4">
          <DialogPrimitive.Title asChild>
            <h3 id={titleId} className="text-base font-semibold text-content">
              {`${t('settings.ai.connectProvider')} ${label}`}
            </h3>
          </DialogPrimitive.Title>
          <p className="mt-0.5 text-xs text-content-muted">{t('settings.ai.oci.description')}</p>
        </div>

        <div className="flex flex-col gap-3" data-testid="oci-settings-form">
          <div className="flex flex-col gap-1.5">
            <Label htmlFor="oci-auth-mode" className="text-xs text-content-secondary">
              {t('settings.ai.oci.authMode')}
            </Label>
            <NativeSelect
              id="oci-auth-mode"
              data-testid="oci-auth-mode"
              value={mode}
              disabled={saving}
              onChange={event => patch({ auth_mode: event.target.value as OciAuthMode })}>
              {OCI_AUTH_MODE_OPTIONS.map(option => (
                <option key={option.value} value={option.value}>
                  {t(option.labelKey)}
                </option>
              ))}
            </NativeSelect>
          </div>

          {mode === 'api_key' && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="oci-api-key" className="text-xs text-content-secondary">
                {t('settings.ai.apiKeyFieldLabel')}
              </Label>
              <TextField
                id="oci-api-key"
                data-testid="oci-api-key"
                type="password"
                {...FIELD_PROPS}
                value={apiKey}
                disabled={saving}
                onChange={event => {
                  setApiKey(event.target.value);
                  setError(null);
                }}
              />
              <p className="text-[11px] text-content-muted">
                {t('settings.ai.oci.apiKeyKeepHint')}
              </p>
            </div>
          )}

          {isOciProfileMode(mode) && (
            <>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="oci-config-file" className="text-xs text-content-secondary">
                  {t('settings.ai.oci.configFile')}
                </Label>
                <TextField
                  id="oci-config-file"
                  data-testid="oci-config-file"
                  mono
                  {...FIELD_PROPS}
                  value={settings.config_file ?? ''}
                  placeholder="~/.oci/config"
                  disabled={saving}
                  onChange={event => patch({ config_file: event.target.value })}
                />
              </div>
              <div className="flex flex-col gap-1.5">
                <Label htmlFor="oci-profile" className="text-xs text-content-secondary">
                  {t('settings.ai.oci.profile')}
                </Label>
                <TextField
                  id="oci-profile"
                  data-testid="oci-profile"
                  mono
                  {...FIELD_PROPS}
                  value={settings.profile ?? ''}
                  placeholder="DEFAULT"
                  disabled={saving}
                  onChange={event => patch({ profile: event.target.value })}
                />
                <p className="text-[11px] text-content-muted">{t('settings.ai.oci.profileHint')}</p>
              </div>
            </>
          )}

          {isOciPrincipalMode(mode) && (
            <p className="text-[11px] text-content-muted" data-testid="oci-principal-hint">
              {t('settings.ai.oci.principalHint')}
              {isOciGatedPrincipalMode(mode) ? ` ${t('settings.ai.oci.principalBuildHint')}` : ''}
            </p>
          )}

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="oci-region" className="text-xs text-content-secondary">
              {t('settings.ai.oci.region')}
            </Label>
            <TextField
              id="oci-region"
              data-testid="oci-region"
              mono
              {...FIELD_PROPS}
              value={settings.region ?? ''}
              placeholder="us-chicago-1"
              disabled={saving}
              onChange={event => patch({ region: event.target.value })}
            />
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="oci-transport" className="text-xs text-content-secondary">
              {t('settings.ai.oci.transport')}
            </Label>
            <NativeSelect
              id="oci-transport"
              data-testid="oci-transport"
              value={settings.transport}
              disabled={saving}
              onChange={event =>
                patch({ transport: event.target.value as OciSettings['transport'] })
              }>
              {OCI_TRANSPORT_OPTIONS.map(option => (
                <option key={option.value} value={option.value}>
                  {t(option.labelKey)}
                </option>
              ))}
            </NativeSelect>
          </div>

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="oci-compartment" className="text-xs text-content-secondary">
              {t('settings.ai.oci.compartment')}
            </Label>
            <TextField
              id="oci-compartment"
              data-testid="oci-compartment"
              mono
              {...FIELD_PROPS}
              value={settings.compartment_id ?? ''}
              placeholder="ocid1.compartment.oc1..…"
              disabled={saving}
              onChange={event => patch({ compartment_id: event.target.value })}
            />
          </div>

          {ociTransportUsesProject(settings.transport) && (
            <div className="flex flex-col gap-1.5">
              <Label htmlFor="oci-project" className="text-xs text-content-secondary">
                {t('settings.ai.oci.project')}
              </Label>
              <TextField
                id="oci-project"
                data-testid="oci-project"
                mono
                {...FIELD_PROPS}
                value={settings.project_id ?? ''}
                disabled={saving}
                onChange={event => patch({ project_id: event.target.value })}
              />
            </div>
          )}

          <div className="flex flex-col gap-1.5">
            <Label htmlFor="oci-dedicated" className="text-xs text-content-secondary">
              {t('settings.ai.oci.dedicatedEndpoint')}
            </Label>
            <TextField
              id="oci-dedicated"
              data-testid="oci-dedicated"
              mono
              {...FIELD_PROPS}
              value={settings.dedicated_endpoint_id ?? ''}
              disabled={saving}
              onChange={event => patch({ dedicated_endpoint_id: event.target.value })}
            />
          </div>

          {error ? <ProviderSetupErrorNotice error={error} /> : null}
        </div>

        <div className="mt-6 flex justify-end gap-2">
          <Button type="button" variant="secondary" size="sm" onClick={onCancel} disabled={saving}>
            {t('common.cancel')}
          </Button>
          <Button
            type="button"
            variant="primary"
            size="sm"
            onClick={() => void handleSave()}
            disabled={saving}
            data-testid="oci-save">
            {saving ? t('settings.ai.saving') : t('common.save')}
          </Button>
        </div>
      </DialogContent>
    </DialogRoot>
  );
};

export default OciSettingsDialog;
