import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import SettingsPanel from '../layout/SettingsPanel';
import MascotPanel from './MascotPanel';
import PersonaPanel from './PersonaPanel';

type TabId = 'personality' | 'face';

const TAB_HASH: Record<TabId, string> = { personality: '', face: '#face' };

const hashToTab = (hash: string): TabId => (hash === '#face' ? 'face' : 'personality');

/**
 * Single Settings entry for the assistant's character. Combines the persona
 * editor (PersonaPanel) and the face/mascot picker (MascotPanel, previously
 * the separate /settings/mascot page) as two tabs under one header. The
 * active tab is reflected in the URL hash (`#face`) so deep links and the
 * legacy persona/mascot redirects land on the right view.
 */
const PersonalityPanel = () => {
  const { t } = useT();
  const location = useLocation();
  const navigate = useNavigate();
  // The router is the single source of truth for the active tab.
  const tab: TabId = hashToTab(location.hash);

  const selectTab = (next: TabId) => {
    navigate(`${location.pathname}${TAB_HASH[next]}`, { replace: true });
  };

  return (
    <SettingsPanel<TabId>
      description={t('settings.personalityFace.menuDesc')}
      tabsAriaLabel={t('settings.personalityFace.title')}
      tabsTestIdPrefix="personality-tab"
      value={tab}
      onChange={selectTab}
      tabs={[
        {
          id: 'personality',
          label: t('settings.assistant.personality'),
          content: <PersonaPanel embedded />,
        },
        {
          id: 'face',
          label: t('settings.assistant.faceMascot'),
          content: <MascotPanel embedded />,
        },
      ]}
    />
  );
};

export default PersonalityPanel;
