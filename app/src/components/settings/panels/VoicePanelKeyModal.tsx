import { useRef, useState } from 'react';

import type { VoiceInstallStatus } from '../../../services/api/voiceInstallApi';
import { testVoiceProvider } from '../../../services/api/voiceSettingsApi';
import { Alert, Button, ModalShell } from '../../ui';
import { SettingsSelect, SettingsTextField } from '../controls';
import { BUILTIN_VOICE_PROVIDER_META } from './VoicePanelProviderChips';

/**
 * Map an install status snapshot to a button label. Single source of truth
 * for the four states the UI surfaces: Not installed / Install / Installing
 * N% / Reinstall.
 */
const installButtonLabel = (
  t: (key: string) => string,
  status: VoiceInstallStatus | null,
  busy: boolean
): string => {
  // Render based on the remote status — the install RPC is fire-and-forget,
  // so the local `busy` flag only covers the brief moment between click and
  // the RPC return. The real "is install running?" signal comes from the
  // polled status table, which lags behind by at most one 2s tick.
  if (status?.state === 'installing') {
    const pct =
      typeof status.progress === 'number' ? `${status.progress}%` : t('voice.providers.ellipsis');
    return `${t('voice.providers.installing')} ${pct}`;
  }
  if (busy) return t('voice.providers.installingBusy');
  if (status?.state === 'installed') return t('voice.providers.reinstallLocally');
  if (status?.state === 'broken') return t('voice.providers.repair');
  if (status?.state === 'error') return t('voice.providers.retryLocally');
  return t('voice.providers.installLocally');
};

const installStatusText = (
  t: (key: string) => string,
  status: VoiceInstallStatus | null,
  ready: boolean
): string => {
  if (status?.state === 'installing') {
    const progress =
      typeof status.progress === 'number'
        ? `${t('voice.providers.installing')} ${status.progress}%`
        : t('voice.providers.installing');
    return status.stage ? `${progress} · ${status.stage}` : progress;
  }
  if (ready) return t('voice.providers.installed');
  if (status?.state === 'error' || status?.state === 'broken') {
    return status.error_detail ?? t('voice.providers.installFailed');
  }
  return t('voice.providers.notInstalled');
};

const installStatusClassName = (status: VoiceInstallStatus | null, ready: boolean): string => {
  if (status?.state === 'error' || status?.state === 'broken') {
    return 'text-coral-600 dark:text-coral-300';
  }
  if (status?.state === 'installing') return 'text-amber-600 dark:text-amber-300';
  if (ready) return 'text-sage-600 dark:text-sage-300';
  return 'text-content-muted';
};

interface VoicePanelKeyModalProps {
  t: (key: string) => string;
  pendingKeySlug: string;
  setPendingKeySlug: (slug: string | null) => void;
  pendingKeyValue: string;
  setPendingKeyValue: (value: string) => void;
  isSavingPendingKey: boolean;
  handleEnableExternalProvider: (slug: string, apiKey: string) => Promise<void>;
  ttsVoice: string;
  setTtsVoice: (value: string) => void;
  piperVoicePresets: ReadonlyArray<{ id: string; label: string }>;
  piperVoicePresetIds: readonly string[];
  piperInstall: VoiceInstallStatus | null;
  isInstallingPiper: boolean;
  handleInstallPiper: () => Promise<void>;
  piperReady: boolean;
  pendingLocalProviderReady: boolean;
  isSavingProviders: boolean;
  onTtsProviderChange: (next: string) => void;
  persistProviders: (update: { tts_voice?: string }) => Promise<void>;
}

/** Inline API-key / Piper-install modal opened from a provider chip. */
const VoicePanelKeyModal = ({
  t,
  pendingKeySlug,
  setPendingKeySlug,
  pendingKeyValue,
  setPendingKeyValue,
  isSavingPendingKey,
  handleEnableExternalProvider,
  ttsVoice,
  setTtsVoice,
  piperVoicePresets,
  piperVoicePresetIds,
  piperInstall,
  isInstallingPiper,
  handleInstallPiper,
  piperReady,
  pendingLocalProviderReady,
  isSavingProviders,
  onTtsProviderChange,
  persistProviders,
}: VoicePanelKeyModalProps) => {
  const [isTestingKey, setIsTestingKey] = useState(false);
  const [keyTestResult, setKeyTestResult] = useState<{ ok: boolean; detail: string } | null>(null);
  // Monotonic id for the in-flight key test. The API-key field stays editable
  // during a test (it is only disabled while *saving*), so without this a
  // result for key A can land next to key B and read as a validation of B —
  // the same "the UI is telling you something untrue about this key" failure
  // this modal is being fixed for. Bumped on every edit and every new test;
  // a response whose id is stale is dropped. Same guard as the LLM routing
  // dialog's `testRequestIdRef`.
  const testRequestIdRef = useRef(0);
  const isPiper = pendingKeySlug === 'piper';

  const close = () => {
    if (isSavingPendingKey) return;
    setPendingKeySlug(null);
    setPendingKeyValue('');
    setKeyTestResult(null);
  };

  return (
    <ModalShell
      titleId="voice-provider-key-title"
      title={
        isPiper
          ? `${t('voice.modal.title')} ${t('voice.providers.chip.piper')}`
          : `${t('voice.modal.title')} ${BUILTIN_VOICE_PROVIDER_META[pendingKeySlug]?.label ?? pendingKeySlug}`
      }
      subtitle={isPiper ? t('voice.modal.piperDesc') : t('voice.modal.desc')}
      onClose={close}
      maxWidthClassName="max-w-md"
      contentClassName="px-5 py-4 space-y-4"
      footer={
        isPiper ? (
          <div className="flex items-center justify-between pt-2">
            <Button
              type="button"
              variant="secondary"
              size="xs"
              onClick={() => {
                setPendingKeySlug(null);
                setKeyTestResult(null);
              }}>
              {t('common.cancel')}
            </Button>
            <Button
              type="button"
              variant="primary"
              size="xs"
              onClick={() => {
                if (!pendingLocalProviderReady) return;
                onTtsProviderChange('piper');
                if (ttsVoice) void persistProviders({ tts_voice: ttsVoice });
                setPendingKeySlug(null);
                setKeyTestResult(null);
              }}
              disabled={!pendingLocalProviderReady || isSavingProviders}>
              {t('voice.modal.enable')}
            </Button>
          </div>
        ) : (
          <div className="flex items-center justify-between pt-2">
            <Button
              type="button"
              variant="secondary"
              size="xs"
              onClick={close}
              disabled={isSavingPendingKey}>
              {t('common.cancel')}
            </Button>

            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant="secondary"
                size="xs"
                disabled={!pendingKeyValue.trim() || isTestingKey || isSavingPendingKey}
                onClick={async () => {
                  if (!pendingKeySlug || !pendingKeyValue.trim()) return;
                  const requestId = testRequestIdRef.current + 1;
                  testRequestIdRef.current = requestId;
                  setIsTestingKey(true);
                  setKeyTestResult(null);
                  try {
                    // Test is a DRY RUN. It must not call
                    // `handleEnableExternalProvider`: that writes the key to
                    // the keychain and activates the provider before it is
                    // known to work, and it clears `pendingKeySlug`, which
                    // unmounts this modal — so the result below would be set
                    // on a dead component and the user would never see it
                    // (#5896). The candidate key goes to the core for
                    // validation only; "Save & Enable" remains the one way to
                    // commit it.
                    const meta = BUILTIN_VOICE_PROVIDER_META[pendingKeySlug];
                    const workload = meta?.capability === 'tts' ? 'tts' : 'stt';
                    const result = await testVoiceProvider(
                      workload as 'stt' | 'tts',
                      pendingKeySlug,
                      true,
                      pendingKeyValue
                    );
                    if (testRequestIdRef.current !== requestId) return;
                    setKeyTestResult(result);
                  } catch (err) {
                    if (testRequestIdRef.current !== requestId) return;
                    setKeyTestResult({
                      ok: false,
                      detail: err instanceof Error ? err.message : 'Test failed',
                    });
                  } finally {
                    // Unconditional: the Test button is disabled while
                    // `isTestingKey`, so there is only ever one test in
                    // flight and this cannot strand the button on "Testing…".
                    setIsTestingKey(false);
                  }
                }}>
                {isTestingKey ? t('voice.modal.testing') : t('voice.modal.testKey')}
              </Button>
              <Button
                type="button"
                variant="primary"
                size="xs"
                onClick={() => void handleEnableExternalProvider(pendingKeySlug, pendingKeyValue)}
                disabled={!pendingKeyValue.trim() || isSavingPendingKey}>
                {isSavingPendingKey ? t('common.loading') : t('voice.modal.saveAndEnable')}
              </Button>
            </div>
          </div>
        )
      }>
      <div data-testid="voice-provider-key-modal" className="space-y-4">
        {isPiper ? (
          <>
            <label className="block space-y-1">
              <span className="text-xs font-medium text-content-muted dark:text-content-secondary">
                {t('voice.providers.piperVoice')}
              </span>
              <SettingsSelect
                value={piperVoicePresetIds.some(v => v === ttsVoice) ? ttsVoice : '__custom__'}
                onChange={e => {
                  if (e.target.value !== '__custom__') setTtsVoice(e.target.value);
                }}
                className="w-full">
                {piperVoicePresets.map(v => (
                  <option key={v.id} value={v.id}>
                    {v.label}
                  </option>
                ))}
                <option value="__custom__">{t('voice.providers.customVoiceOption')}</option>
              </SettingsSelect>
            </label>

            <div className="flex items-center gap-2">
              <Button
                type="button"
                variant={piperReady ? 'secondary' : 'primary'}
                size="xs"
                onClick={() => void handleInstallPiper()}
                disabled={isInstallingPiper || piperInstall?.state === 'installing'}>
                {installButtonLabel(t, piperInstall, isInstallingPiper)}
              </Button>
              <span className={`text-[11px] ${installStatusClassName(piperInstall, piperReady)}`}>
                {installStatusText(t, piperInstall, piperReady)}
              </span>
            </div>
          </>
        ) : (
          <>
            <label className="block space-y-1">
              <span className="text-xs font-medium text-content-muted dark:text-content-secondary">
                {t('voice.providers.chip.apiKeyLabel')}
              </span>
              <SettingsTextField
                id="voice-provider-key-input"
                type="password"
                autoComplete="off"
                autoCorrect="off"
                spellCheck={false}
                data-form-type="other"
                data-lpignore="true"
                value={pendingKeyValue}
                onChange={e => {
                  setPendingKeyValue(e.target.value);
                  setKeyTestResult(null);
                  // Any edit invalidates a test still in flight for the old
                  // key, so its result cannot arrive and describe this one.
                  testRequestIdRef.current += 1;
                }}
                disabled={isSavingPendingKey}
                placeholder={t('voice.providers.chip.apiKeyPlaceholder')}
                className="w-full"
              />
            </label>

            {keyTestResult && (
              <Alert
                variant={keyTestResult.ok ? 'success' : 'destructive'}
                className="rounded-md px-3 py-2 text-xs">
                {keyTestResult.detail}
              </Alert>
            )}
          </>
        )}
      </div>
    </ModalShell>
  );
};

export default VoicePanelKeyModal;
