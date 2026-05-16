import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  openhumanGetComposioTriggerSettings,
  openhumanUpdateComposioTriggerSettings,
} from '../../../utils/tauriCommands';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const ComposioTriagePanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  const [triageDisabled, setTriageDisabled] = useState(false);
  const [disabledToolkits, setDisabledToolkits] = useState('');
  const [loading, setLoading] = useState(true);
  const [saving, setSaving] = useState(false);
  const [saveStatus, setSaveStatus] = useState<'idle' | 'saved' | 'error'>('idle');
  const saveStatusTimer = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    let isMounted = true;
    openhumanGetComposioTriggerSettings()
      .then(res => {
        if (!isMounted) return;
        const settings = res.result;
        if (!settings) return;
        setTriageDisabled(settings.triage_disabled ?? false);
        setDisabledToolkits((settings.triage_disabled_toolkits ?? []).join(', '));
      })
      .catch(err => {
        if (!isMounted) return;
        console.warn('[ComposioTriagePanel] failed to load settings:', err);
      })
      .finally(() => {
        if (isMounted) setLoading(false);
      });

    return () => {
      isMounted = false;
      if (saveStatusTimer.current !== null) {
        clearTimeout(saveStatusTimer.current);
      }
    };
  }, []);

  const handleSave = async () => {
    setSaving(true);
    try {
      const toolkitList = disabledToolkits
        .split(',')
        .map(e => e.trim().toLowerCase())
        .filter(Boolean);
      await openhumanUpdateComposioTriggerSettings({
        triage_disabled: triageDisabled,
        triage_disabled_toolkits: toolkitList,
      });
      setSaveStatus('saved');
      if (saveStatusTimer.current !== null) {
        clearTimeout(saveStatusTimer.current);
      }
      saveStatusTimer.current = setTimeout(() => setSaveStatus('idle'), 3000);
    } catch (err) {
      console.warn('[ComposioTriagePanel] failed to save settings:', err);
      if (saveStatusTimer.current !== null) {
        clearTimeout(saveStatusTimer.current);
        saveStatusTimer.current = null;
      }
      setSaveStatus('error');
    } finally {
      setSaving(false);
    }
  };

  if (loading) {
    return (
      <div>
        <SettingsHeader
          title={t('composio.triageTitle')}
          showBackButton
          onBack={navigateBack}
          breadcrumbs={breadcrumbs}
        />
        <div className="p-4">
          <p className="text-sm text-stone-500">Loading…</p>
        </div>
      </div>
    );
  }

  return (
    <div>
      <SettingsHeader
        title={t('composio.triageTitle')}
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4 space-y-5">
        <p className="text-sm text-stone-500">
          {t('composio.triageDesc')}{' '}
          <span className="font-mono">OPENHUMAN_TRIGGER_TRIAGE_DISABLED</span>{' '}
          {t('composio.envVarOverrides')}
        </p>

        <div className="rounded-2xl border border-stone-200 bg-stone-50/60 p-4 space-y-1">
          <button
            type="button"
            role="switch"
            aria-checked={triageDisabled}
            aria-label="Disable AI triage for all triggers"
            onClick={() => setTriageDisabled(v => !v)}
            className="w-full flex items-center justify-between">
            <div className="text-left">
              <span className="text-sm font-medium text-stone-900">
                {t('composio.disableAllTriage')}
              </span>
              <p className="text-xs text-stone-500 mt-0.5">{t('composio.triggersStillRecorded')}</p>
            </div>
            <div
              className={`ml-3 flex-shrink-0 w-9 h-5 rounded-full transition-colors relative ${
                triageDisabled ? 'bg-coral-400' : 'bg-stone-200'
              }`}>
              <div
                className={`absolute top-0.5 w-4 h-4 rounded-full bg-white shadow transition-transform ${
                  triageDisabled ? 'translate-x-4' : 'translate-x-0.5'
                }`}
              />
            </div>
          </button>
        </div>

        <div className={`space-y-2 ${triageDisabled ? 'opacity-40 pointer-events-none' : ''}`}>
          <label className="block text-sm font-medium text-stone-800" htmlFor="disabled-toolkits">
            {t('composio.disableSpecificIntegrations')}
          </label>
          <p className="text-xs text-stone-500">
            Comma-separated integration slugs, e.g. <span className="font-mono">gmail, slack</span>.
            Case-insensitive.
          </p>
          <input
            id="disabled-toolkits"
            type="text"
            value={disabledToolkits}
            onChange={e => setDisabledToolkits(e.target.value)}
            placeholder="gmail, slack, ..."
            disabled={triageDisabled}
            className="w-full rounded-xl border border-stone-200 bg-white px-3 py-2 text-sm text-stone-900 placeholder-stone-400 focus:border-primary-400 focus:outline-none focus:ring-1 focus:ring-primary-400 disabled:cursor-not-allowed"
          />
        </div>

        <button
          type="button"
          onClick={handleSave}
          disabled={saving}
          className="w-full py-2 rounded-xl bg-primary-600 text-white text-sm font-medium hover:bg-primary-500 transition-colors disabled:opacity-50">
          {saving ? t('common.loading') : t('common.save')}
        </button>

        {saveStatus === 'saved' && (
          <p className="text-xs text-center text-green-600">{t('composio.settingsSaved')}</p>
        )}
        {saveStatus === 'error' && (
          <p className="text-xs text-center text-red-500">{t('composio.saveFailed')}</p>
        )}
      </div>
    </div>
  );
};

export default ComposioTriagePanel;
