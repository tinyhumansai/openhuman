import SettingsHeader from '../components/SettingsHeader';
import SettingsMenuItem from '../components/SettingsMenuItem';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const developerItems = [
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
    id: 'memory-data',
    title: 'Memory Data',
    description: 'Knowledge graph, insights, activity heatmap, and file management',
    route: 'memory-data',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9.663 17h4.673M12 3v1m6.364 1.636l-.707.707M21 12h-1M4 12H3m3.343-5.657l-.707-.707m2.828 9.9a5 5 0 117.072 0l-.548.547A3.374 3.374 0 0014 18.469V19a2 2 0 11-4 0v-.531c0-.895-.356-1.754-.988-2.386l-.548-.547z"
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
  const { navigateToSettings, navigateBack, breadcrumbs } = useSettingsNavigation();

  return (
    <div className="z-10 relative">
      <SettingsHeader
        title="Developer Options"
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

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
