import { useT } from '../../../lib/i18n/I18nContext';

interface ConfigureLaterCalloutProps {
  /** Settings-route the user will be deep-linked to after onboarding ends. */
  settingsHref: string;
}

/**
 * Placeholder rendered inside a CustomWizardStep when the user picks
 * "Configure" but we don't have inline controls for that domain yet.
 * Records the user's intent; final wiring lives in the matching Settings
 * panel, surfaced via `settingsHref` once onboarding completes.
 */
const ConfigureLaterCallout = ({ settingsHref }: ConfigureLaterCalloutProps) => {
  const { t } = useT();
  return (
    <div className="flex flex-col gap-3">
      <p className="text-xs text-stone-700 leading-relaxed">
        {t('onboarding.custom.configureLater')}
      </p>
      <p className="text-[11px] text-stone-500" data-testid="configure-later-hint">
        {t('onboarding.custom.openSettings')}:{' '}
        <code className="text-stone-700">{settingsHref}</code>
      </p>
    </div>
  );
};

export default ConfigureLaterCallout;
