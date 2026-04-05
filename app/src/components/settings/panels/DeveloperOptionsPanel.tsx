import SettingsHeader from '../components/SettingsHeader';
import SettingsMenuItem from '../components/SettingsMenuItem';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const developerItems = [
  {
    id: 'skills',
    title: 'Skills',
    description: 'Configure Slack, Discord, and other skills',
    route: 'skills',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9.75 3a.75.75 0 00-1.5 0v2.25H6a2.25 2.25 0 000 4.5h2.25V12H6a2.25 2.25 0 000 4.5h2.25V18a.75.75 0 001.5 0v-1.5H12V18a.75.75 0 001.5 0v-1.5H18a2.25 2.25 0 000-4.5h-4.5V9.75H18a2.25 2.25 0 000-4.5h-4.5V3a.75.75 0 00-1.5 0v2.25H9.75V3z"
        />
      </svg>
    ),
  },
  {
    id: 'ai',
    title: 'AI Configuration',
    description: 'Configure SOUL persona and AI behavior',
    route: 'ai',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9.75 3a.75.75 0 00-1.5 0v2.25H6a2.25 2.25 0 000 4.5h2.25V12H6a2.25 2.25 0 000 4.5h2.25V18a.75.75 0 001.5 0v-1.5H12V18a.75.75 0 001.5 0v-1.5H18a2.25 2.25 0 000-4.5h-4.5V9.75H18a2.25 2.25 0 000-4.5h-4.5V3a.75.75 0 00-1.5 0v2.25H9.75V3z"
        />
      </svg>
    ),
  },
  {
    id: 'tauri-commands',
    title: 'Tauri Command Console',
    description: 'Run OpenHuman Tauri commands for quick testing',
    route: 'tauri-commands',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9 12h6m2 8H7a2 2 0 01-2-2V6a2 2 0 012-2h6l6 6v8a2 2 0 01-2 2z"
        />
      </svg>
    ),
  },
  {
    id: 'webhooks-debug',
    title: 'Webhooks',
    description: 'Inspect runtime webhook registrations and captured request logs',
    route: 'webhooks-debug',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M13.828 10.172a4 4 0 010 5.656l-2 2a4 4 0 01-5.656-5.656l1-1m5-5a4 4 0 015.656 5.656l-1 1m-5 5l5-5"
        />
      </svg>
    ),
  },
  {
    id: 'memory-debug',
    title: 'Memory Debug',
    description: 'Inspect memory documents, namespaces, and test query/recall',
    route: 'memory-debug',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9 12h6m2 8H7a2 2 0 01-2-2V6a2 2 0 012-2h6l6 6v8a2 2 0 01-2 2z"
        />
      </svg>
    ),
  },
];

const DeveloperOptionsPanel = () => {
  const { navigateToSettings, navigateBack } = useSettingsNavigation();

  return (
    <div className="z-10 relative">
      <SettingsHeader title="Developer Options" showBackButton={true} onBack={navigateBack} />

      <div>
        {developerItems.map((item, index) => (
          <SettingsMenuItem
            key={item.id}
            icon={item.icon}
            title={item.title}
            description={item.description}
            onClick={() => navigateToSettings(item.route)}
            isFirst={index === 0}
            isLast={index === developerItems.length - 1}
          />
        ))}
      </div>
    </div>
  );
};

export default DeveloperOptionsPanel;
