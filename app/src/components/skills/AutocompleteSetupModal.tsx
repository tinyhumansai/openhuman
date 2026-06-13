/**
 * Text Auto-Complete setup/enable modal.
 *
 * Simple enable flow: shows current state, lets user enable with one click,
 * and shows a success confirmation — matching the UX of the Screen
 * Intelligence setup modal.
 */
import { useState } from 'react';
import { useNavigate } from 'react-router-dom';

import { useT } from '../../lib/i18n/I18nContext';
import { useCoreState } from '../../providers/CoreStateProvider';
import {
  openhumanAutocompleteSetStyle,
  openhumanAutocompleteStart,
} from '../../utils/tauriCommands/autocomplete';
import {
  SetupNotice,
  SetupSettingRow,
  SetupSuccess,
  SkillSetupModalShell,
} from './SkillSetupPrimitives';

type Step = 'enable' | 'success';

interface Props {
  onClose: () => void;
}

export default function AutocompleteSetupModal({ onClose }: Props) {
  const navigate = useNavigate();
  const { t } = useT();
  const { snapshot, refresh } = useCoreState();
  const status = snapshot.runtime.autocomplete;

  const [step, setStep] = useState<Step>('enable');
  const [isEnabling, setIsEnabling] = useState(false);
  const [enableError, setEnableError] = useState<string | null>(null);

  const handleEnable = async () => {
    setIsEnabling(true);
    setEnableError(null);
    try {
      // Enable in config
      await openhumanAutocompleteSetStyle({ enabled: true });
      // Start the service
      await openhumanAutocompleteStart();
      await refresh();
      setStep('success');
    } catch (error) {
      setEnableError(
        error instanceof Error ? error.message : t('skills.setup.autocomplete.enableError')
      );
    } finally {
      setIsEnabling(false);
    }
  };

  const handleGoToSettings = () => {
    onClose();
    navigate('/settings/autocomplete');
  };

  return (
    <SkillSetupModalShell
      onClose={onClose}
      title={t('skills.setup.autocomplete.title')}
      titleId="ac-setup-title"
      subtitle={
        <>
          {step === 'enable' && t('skills.setup.autocomplete.stepEnable')}
          {step === 'success' && t('skills.setup.autocomplete.stepSuccess')}
        </>
      }
      icon={
        <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
          <path
            strokeLinecap="round"
            strokeLinejoin="round"
            strokeWidth={1.8}
            d="M4 7h16M4 12h10m-10 5h7m10 0l3 3m0 0l3-3m-3 3v-8"
          />
        </svg>
      }>
      {/* ─── Enable step ─── */}
      {step === 'enable' && (
        <div className="space-y-4">
          <p className="text-xs text-stone-500 dark:text-neutral-400 leading-relaxed">
            {t('skills.setup.autocomplete.description')}
          </p>

          {!status?.platform_supported && status !== null && (
            <SetupNotice tone="amber">{t('skills.setup.autocomplete.notSupported')}</SetupNotice>
          )}

          <div className="space-y-2">
            <SetupSettingRow
              label={t('skills.setup.autocomplete.stylePreset')}
              value={t('skills.setup.autocomplete.stylePresetValue')}
            />
            <SetupSettingRow label={t('skills.setup.autocomplete.acceptKey')} value="Tab" mono />
            <SetupSettingRow
              label={t('skills.setup.autocomplete.debounce')}
              value={`${status?.debounce_ms ?? 120}ms`}
            />
          </div>

          {enableError && <SetupNotice tone="coral">{enableError}</SetupNotice>}

          <button
            type="button"
            onClick={() => void handleEnable()}
            disabled={isEnabling || (status !== null && !status.platform_supported)}
            className="w-full rounded-xl bg-primary-500 px-4 py-2.5 text-sm font-medium text-white hover:bg-primary-600 disabled:opacity-50 transition-colors">
            {isEnabling
              ? t('skills.setup.autocomplete.enabling')
              : t('skills.setup.autocomplete.enableBtn')}
          </button>
        </div>
      )}

      {/* ─── Success step ─── */}
      {step === 'success' && (
        <SetupSuccess
          title={t('skills.setup.autocomplete.activeTitle')}
          description={t('skills.setup.autocomplete.activeDesc')}
          settingsLabel={t('skills.setup.autocomplete.customizeSettings')}
          finishLabel={t('common.finish')}
          onSettings={handleGoToSettings}
          onFinish={onClose}
        />
      )}
    </SkillSetupModalShell>
  );
}
