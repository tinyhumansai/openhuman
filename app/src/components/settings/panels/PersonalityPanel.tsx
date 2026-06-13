import { useLocation, useNavigate } from 'react-router-dom';

import { useT } from '../../../lib/i18n/I18nContext';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
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
  const { navigateBack, breadcrumbs } = useSettingsNavigation();
  const location = useLocation();
  const navigate = useNavigate();
  // The router is the single source of truth for the active tab.
  const tab: TabId = hashToTab(location.hash);

  const selectTab = (next: TabId) => {
    navigate(`${location.pathname}${TAB_HASH[next]}`, { replace: true });
  };

  const tabs: { id: TabId; label: string }[] = [
    { id: 'personality', label: t('settings.assistant.personality') },
    { id: 'face', label: t('settings.assistant.faceMascot') },
  ];

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title={t('settings.personalityFace.title')}
        showBackButton
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div
        role="tablist"
        aria-label={t('settings.personalityFace.title')}
        className="flex gap-1 px-4 pt-3 border-b border-neutral-200 dark:border-neutral-800">
        {tabs.map(({ id, label }) => {
          const selected = tab === id;
          return (
            <button
              key={id}
              type="button"
              role="tab"
              aria-selected={selected}
              data-testid={`personality-tab-${id}`}
              onClick={() => selectTab(id)}
              className={`px-3 py-2 text-sm font-medium border-b-2 -mb-px transition-colors ${
                selected
                  ? 'border-primary-500 text-neutral-800 dark:text-neutral-100'
                  : 'border-transparent text-neutral-500 dark:text-neutral-400 hover:text-neutral-700 dark:hover:text-neutral-200'
              }`}>
              {label}
            </button>
          );
        })}
      </div>

      {tab === 'personality' ? <PersonaPanel embedded /> : <MascotPanel embedded />}
    </div>
  );
};

export default PersonalityPanel;
