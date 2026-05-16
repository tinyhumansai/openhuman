import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { setCloudProviderKey } from '../../../services/api/aiSettingsApi';
import OnboardingNextButton from '../components/OnboardingNextButton';

interface ApiKeysStepProps {
  onNext: () => void;
  onSkip: () => void;
}

const ApiKeysStep = ({ onNext, onSkip }: ApiKeysStepProps) => {
  const { t } = useT();
  const [openai, setOpenai] = useState('');
  const [anthropic, setAnthropic] = useState('');
  const [saving, setSaving] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const handleSave = async () => {
    const trimmedOpenai = openai.trim();
    const trimmedAnthropic = anthropic.trim();
    if (!trimmedOpenai && !trimmedAnthropic) {
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
      className="rounded-2xl bg-white p-10 shadow-soft animate-fade-up">
      <div className="text-center">
        <h1 className="text-2xl font-display text-stone-900 mb-2 leading-tight">
          {t('onboarding.apiKeys.title')}
        </h1>
        <p className="text-stone-500 text-sm leading-relaxed">{t('onboarding.apiKeys.subtitle')}</p>
      </div>

      <div className="mt-6 flex flex-col gap-4">
        <div className="flex flex-col gap-1.5">
          <label htmlFor="onboarding-openai-key" className="text-xs font-medium text-stone-700">
            {t('onboarding.apiKeys.openaiLabel')}
          </label>
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
            className="rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm text-stone-900 placeholder-stone-400 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
          />
        </div>

        <div className="flex flex-col gap-1.5">
          <label htmlFor="onboarding-anthropic-key" className="text-xs font-medium text-stone-700">
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
            className="rounded-lg border border-stone-300 bg-white px-3 py-2 text-sm text-stone-900 placeholder-stone-400 focus:border-primary-500 focus:outline-none focus:ring-1 focus:ring-primary-500"
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
          className="text-xs text-stone-500 hover:text-stone-700 underline disabled:opacity-50">
          {t('onboarding.apiKeys.skipForNow')}
        </button>
      </div>
    </div>
  );
};

export default ApiKeysStep;
