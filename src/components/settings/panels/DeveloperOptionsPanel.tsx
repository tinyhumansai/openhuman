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
    id: 'local-model',
    title: 'Local Model Runtime',
    description: 'Monitor download/load status and test local model calls',
    route: 'local-model',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9 17v-6m3 6V7m3 10v-4m5 6H4a2 2 0 01-2-2V7a2 2 0 012-2h16a2 2 0 012 2v10a2 2 0 01-2 2z"
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
    <div className="overflow-hidden h-full flex flex-col z-10 relative">
      <SettingsHeader title="Developer Options" showBackButton={true} onBack={navigateBack} />

      <div className="flex-1 overflow-y-auto max-w-md mx-auto">
        <div className="p-4">
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
      </div>
    </div>
  );
};

export default DeveloperOptionsPanel;
