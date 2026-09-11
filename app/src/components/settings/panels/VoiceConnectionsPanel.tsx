import { useT } from '../../../lib/i18n/I18nContext';
import SettingsTabbedPage from '../layout/SettingsTabbedPage';
import VoicePanel from './VoicePanel';

/** Connections → Voice rendered in the shared settings-detail page shell. */
export default function VoiceConnectionsPanel() {
  const { t } = useT();

  return (
    <SettingsTabbedPage
      title={t('pages.settings.ai.voice')}
      description={t('voice.providers.desc')}>
      <VoicePanel embedded scrollable={false} />
    </SettingsTabbedPage>
  );
}
