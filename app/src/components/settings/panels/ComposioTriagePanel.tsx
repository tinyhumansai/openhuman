import { useEffect, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import {
  openhumanGetComposioTriggerSettings,
  openhumanUpdateComposioTriggerSettings,
} from '../../../utils/tauriCommands';
import Button from '../../ui/Button';
import {
  SettingsRow,
  SettingsSection,
  SettingsStatusLine,
  SettingsSwitch,
  SettingsTextField,
} from '../controls';
import SettingsPanel from '../layout/SettingsPanel';

const ComposioTriagePanel = () => {
  const { t } = useT();

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
      <SettingsPanel description={t('settings.developerMenu.composio.desc')}>
        <p className="text-sm text-content-muted">{t('settings.composio.loading')}</p>
      </SettingsPanel>
    );
  }

  return (
    <SettingsPanel description={t('settings.developerMenu.composio.desc')}>
      <p className="text-sm text-content-muted">
        {t('composio.triageDesc')}{' '}
        <span className="font-mono">OPENHUMAN_TRIGGER_TRIAGE_DISABLED</span>{' '}
        {t('composio.envVarOverrides')}
      </p>

      <SettingsSection>
        <SettingsRow
          htmlFor="switch-triage-disabled"
          label={t('composio.disableAllTriage')}
          description={t('composio.triggersStillRecorded')}
          control={
            <SettingsSwitch
              id="switch-triage-disabled"
              checked={triageDisabled}
              onCheckedChange={next => setTriageDisabled(next)}
              aria-label={t('composio.disableAllTriage')}
            />
          }
        />
      </SettingsSection>

      <SettingsSection
        title={t('composio.disableSpecificIntegrations')}
        description={`${t('composio.integrationSlugsHelp')} ${t('composio.integrationSlugsExample')}. ${t('composio.integrationSlugsCaseInsensitive')}`}>
        <SettingsRow
          stacked
          disabled={triageDisabled}
          control={
            <SettingsTextField
              id="disabled-toolkits"
              value={disabledToolkits}
              onChange={e => setDisabledToolkits(e.target.value)}
              placeholder={t('composio.integrationSlugsPlaceholder')}
              disabled={triageDisabled}
              aria-label={t('composio.disableSpecificIntegrations')}
            />
          }
        />
      </SettingsSection>

      <div className="flex items-center gap-3">
        <Button
          type="button"
          variant="primary"
          size="sm"
          onClick={() => void handleSave()}
          disabled={saving}>
          {saving ? t('common.loading') : t('common.save')}
        </Button>
        <SettingsStatusLine
          saving={saving}
          savedNote={saveStatus === 'saved' ? t('composio.settingsSaved') : null}
          error={saveStatus === 'error' ? t('composio.saveFailed') : null}
          savingLabel={t('common.loading')}
        />
      </div>
    </SettingsPanel>
  );
};

export default ComposioTriagePanel;
