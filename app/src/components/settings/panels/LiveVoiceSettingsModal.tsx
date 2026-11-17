import { Check, KeyRound } from 'lucide-react';
import { type ReactNode, useId, useState } from 'react';

import { cn } from '../../../lib/cn';
import { useT } from '../../../lib/i18n/I18nContext';
import type {
  LiveVoiceProvider,
  LiveVoiceSettings,
  LiveVoiceSettingsPatch,
  LiveVoiceTestResult,
} from '../../../services/api/liveVoiceApi';
import Button from '../../ui/Button';
import { ModalShell } from '../../ui/ModalShell';
import NativeSelect from '../../ui/NativeSelect';
import TextField from '../../ui/TextField';
import { IncludedTag } from './LiveVoiceVendorCard';
import LiveVoiceVendorLogo from './LiveVoiceVendorLogo';
import { type LiveVoiceVendor, readSetting, vendorDescKey, voiceFields } from './liveVoiceVendors';

export type LiveVoiceTestState =
  | { kind: 'testing' }
  | { kind: 'done'; result: LiveVoiceTestResult };

export interface LiveVoiceSettingsModalProps {
  vendor: LiveVoiceVendor;
  defaultProvider: string;
  settings: LiveVoiceSettings | null;
  saving: boolean;
  tests: Record<string, LiveVoiceTestState>;
  keyDrafts: Record<string, string>;
  /** Save / error line for the latest write, shown in the modal footer. */
  status: ReactNode;
  onClose: () => void;
  onUse: (providerId: string) => void;
  onTest: (providerId: string) => void;
  onKeyDraft: (providerId: string, value: string) => void;
  onSaveKey: (provider: LiveVoiceProvider) => void;
  onClearKey: (provider: LiveVoiceProvider) => void;
  onPersist: (patch: LiveVoiceSettingsPatch) => void;
}

/**
 * Settings for one voice service: each way to connect it (managed by
 * TinyHumans, or the user's own key) with a Use button and a connectivity
 * test, the key editor, and the service's voice and language.
 */
const LiveVoiceSettingsModal = ({
  vendor,
  defaultProvider,
  settings,
  saving,
  tests,
  keyDrafts,
  status,
  onClose,
  onUse,
  onTest,
  onKeyDraft,
  onSaveKey,
  onClearKey,
  onPersist,
}: LiveVoiceSettingsModalProps) => {
  const { t } = useT();
  const titleId = useId();
  const [replacing, setReplacing] = useState<Record<string, boolean>>({});
  const descKey = vendorDescKey(vendor.id);

  const testLine = (providerId: string) => {
    const test = tests[providerId];
    if (!test) return null;
    if (test.kind === 'testing') {
      return (
        <p className="text-xs text-content-muted" data-testid={`live-voice-test-${providerId}`}>
          {t('connections.voiceAgents.testing')}
        </p>
      );
    }
    const { result } = test;
    return (
      <p
        className={cn(
          'text-xs',
          result.ok ? 'text-sage-700 dark:text-sage-300' : 'break-words text-coral-600'
        )}
        data-testid={`live-voice-test-${providerId}`}
        data-ok={result.ok ? 'true' : 'false'}>
        {result.ok
          ? result.latency_ms != null
            ? t('connections.voiceAgents.testOk').replace('{ms}', String(result.latency_ms))
            : t('connections.voiceAgents.testOkNoLatency')
          : t('connections.voiceAgents.testFailed').replace('{error}', result.error ?? '')}
      </p>
    );
  };

  const keyInput = (provider: LiveVoiceProvider) => {
    const draft = keyDrafts[provider.id] ?? '';
    return (
      <div className="flex flex-wrap items-center gap-2">
        <TextField
          type="password"
          autoComplete="off"
          inputSize="sm"
          className="min-w-0 flex-1"
          aria-label={`${provider.label} ${t('connections.voiceAgents.apiKey')}`}
          placeholder={t('connections.voiceAgents.apiKeyPlaceholder')}
          data-testid={`live-voice-key-${provider.id}`}
          value={draft}
          disabled={saving}
          onChange={e => onKeyDraft(provider.id, e.target.value)}
        />
        <Button
          size="sm"
          analyticsId="live-voice-save-key"
          data-testid={`live-voice-save-key-${provider.id}`}
          disabled={saving || !draft.trim()}
          onClick={() => {
            onSaveKey(provider);
            setReplacing(prev => ({ ...prev, [provider.id]: false }));
          }}>
          {t('connections.voiceAgents.saveKey')}
        </Button>
      </div>
    );
  };

  const keyEditor = (provider: LiveVoiceProvider) => {
    if (provider.kind !== 'byok' || !provider.key_slug) return null;
    if (!provider.configured) {
      return (
        <div className="flex flex-col gap-2">
          <p className="text-xs text-content-muted">{t('connections.voiceAgents.addKeyHint')}</p>
          {keyInput(provider)}
        </div>
      );
    }
    const open = replacing[provider.id] ?? false;
    return (
      <div className="flex flex-col gap-2">
        <div className="flex flex-wrap items-center gap-2">
          <span className="flex min-w-0 flex-1 items-center gap-1.5 text-xs text-content-secondary">
            <KeyRound className="h-3.5 w-3.5 shrink-0" aria-hidden />
            {t('connections.voiceAgents.keyOnFile')}
          </span>
          <Button
            size="xs"
            variant="tertiary"
            analyticsId="live-voice-replace-key"
            data-testid={`live-voice-replace-key-${provider.id}`}
            disabled={saving}
            aria-expanded={open}
            onClick={() => setReplacing(prev => ({ ...prev, [provider.id]: !open }))}>
            {open ? t('common.cancel') : t('connections.voiceAgents.replaceKey')}
          </Button>
          <Button
            size="xs"
            variant="tertiary"
            tone="danger"
            analyticsId="live-voice-clear-key"
            data-testid={`live-voice-clear-key-${provider.id}`}
            disabled={saving}
            onClick={() => onClearKey(provider)}>
            {t('connections.voiceAgents.clearKey')}
          </Button>
        </div>
        {open && keyInput(provider)}
      </div>
    );
  };

  const option = (provider: LiveVoiceProvider) => {
    const inUse = provider.id === defaultProvider;
    const hosted = provider.kind === 'hosted';
    const testing = tests[provider.id]?.kind === 'testing';
    const keyBody = keyEditor(provider);
    const testBody = testLine(provider.id);
    return (
      <div
        key={provider.id}
        data-testid={`live-voice-option-${provider.id}`}
        data-selected={inUse || undefined}
        className={cn(
          'rounded-lg border',
          inUse
            ? 'border-primary-500 bg-primary-50 dark:bg-primary-500/10'
            : 'border-line bg-surface'
        )}>
        <div className="flex flex-wrap items-start gap-x-3 gap-y-2 p-3.5">
          <div className="min-w-0 flex-1 basis-48">
            <div className="flex flex-wrap items-center gap-x-2 gap-y-1">
              <span className="text-sm font-semibold text-content">
                {hosted
                  ? t('connections.voiceAgents.optionManaged')
                  : t('connections.voiceAgents.optionOwnKey')}
              </span>
              {hosted && <IncludedTag />}
            </div>
            <p className="mt-0.5 text-xs leading-relaxed text-content-muted">
              {hosted
                ? t('connections.voiceAgents.optionManagedDesc')
                : t('connections.voiceAgents.optionOwnKeyDesc')}
            </p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {provider.configured && (
              <Button
                size="sm"
                variant="tertiary"
                analyticsId="live-voice-test-provider"
                data-testid={`live-voice-test-button-${provider.id}`}
                disabled={testing}
                onClick={() => onTest(provider.id)}>
                {testing ? t('connections.voiceAgents.testing') : t('connections.voiceAgents.test')}
              </Button>
            )}
            {inUse ? (
              <span
                className="inline-flex h-8 items-center gap-1 px-2 text-xs font-semibold text-primary-600 dark:text-primary-300"
                data-testid={`live-voice-in-use-${provider.id}`}>
                <Check className="h-4 w-4" aria-hidden />
                {t('connections.voiceAgents.badgeInUse')}
              </span>
            ) : provider.configured ? (
              <Button
                size="sm"
                analyticsId="live-voice-use-provider"
                data-testid={`live-voice-use-${provider.id}`}
                disabled={saving}
                onClick={() => onUse(provider.id)}>
                {t('connections.voiceAgents.use')}
              </Button>
            ) : null}
          </div>
        </div>
        {(testBody || keyBody) && (
          <div className="flex flex-col gap-2 border-t border-line-subtle px-3.5 py-3">
            {testBody}
            {keyBody}
          </div>
        )}
      </div>
    );
  };

  const pickers = () => {
    if (!settings) return null;
    // The voice block is shared by every way to reach the service, so read the
    // choices off the provider in use, or the first one that lists any.
    const provider =
      vendor.providers.find(p => p.id === defaultProvider) ??
      vendor.providers.find(p => p.voices.length > 0 || p.languages.length > 0);
    if (!provider) return null;
    const fields = voiceFields(provider.id);
    if (!fields) return null;
    const showVoice = provider.voices.length > 0;
    const showLanguage = fields.language != null && provider.languages.length > 0;
    if (!showVoice && !showLanguage) return null;
    const voiceLabel =
      fields.voice === 'speaker'
        ? t('connections.voiceAgents.speaker')
        : t('connections.voiceAgents.voice');
    return (
      <section className="flex flex-col gap-2.5">
        <h3 className="text-xs font-semibold tracking-wide text-content-muted">
          {t('connections.voiceAgents.voiceSettings')}
        </h3>
        <div className="grid gap-3 sm:grid-cols-2">
          {showVoice && (
            <label className="flex flex-col gap-1.5 text-xs font-medium text-content-secondary">
              <span>{voiceLabel}</span>
              <NativeSelect
                inputSize="sm"
                aria-label={`${vendor.name} ${voiceLabel}`}
                data-testid={`live-voice-voice-${vendor.id}`}
                value={readSetting(settings, fields.block, fields.voice)}
                disabled={saving}
                onChange={e =>
                  onPersist({ [fields.block]: { [fields.voice]: e.target.value || null } })
                }>
                <option value="">{t('connections.voiceAgents.providerDefault')}</option>
                {provider.voices.map(v => (
                  <option key={v} value={v}>
                    {v}
                  </option>
                ))}
              </NativeSelect>
            </label>
          )}
          {showLanguage && fields.language && (
            <label className="flex flex-col gap-1.5 text-xs font-medium text-content-secondary">
              <span>{t('connections.voiceAgents.language')}</span>
              <NativeSelect
                inputSize="sm"
                aria-label={`${vendor.name} ${t('connections.voiceAgents.language')}`}
                data-testid={`live-voice-language-${vendor.id}`}
                value={readSetting(settings, fields.block, fields.language)}
                disabled={saving}
                onChange={e =>
                  onPersist({
                    [fields.block]: { [fields.language as string]: e.target.value || null },
                  })
                }>
                <option value="">{t('connections.voiceAgents.providerDefault')}</option>
                {provider.languages.map(l => (
                  <option key={l} value={l}>
                    {l}
                  </option>
                ))}
              </NativeSelect>
            </label>
          )}
        </div>
      </section>
    );
  };

  return (
    <ModalShell
      onClose={onClose}
      title={vendor.name}
      titleId={titleId}
      subtitle={descKey ? t(descKey) : undefined}
      icon={<LiveVoiceVendorLogo vendorId={vendor.id} className="h-5 w-5" />}
      maxWidthClassName="max-w-lg"
      contentClassName="flex flex-col gap-5 px-5 py-4"
      testId="live-voice-modal"
      footer={
        <div className="flex w-full items-center justify-between gap-3">
          <div className="min-w-0 flex-1">{status}</div>
          <Button
            size="sm"
            variant="secondary"
            analyticsId="live-voice-modal-close"
            data-testid="live-voice-modal-close"
            onClick={onClose}>
            {t('common.close')}
          </Button>
        </div>
      }>
      <section className="flex flex-col gap-2.5">
        <h3 className="text-xs font-semibold tracking-wide text-content-muted">
          {t('connections.voiceAgents.connection')}
        </h3>
        <div className="flex flex-col gap-2">{vendor.providers.map(option)}</div>
      </section>
      {pickers()}
    </ModalShell>
  );
};

export default LiveVoiceSettingsModal;
