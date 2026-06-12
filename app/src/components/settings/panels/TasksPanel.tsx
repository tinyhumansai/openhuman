import { useT } from '../../../lib/i18n/I18nContext';
import IntelligenceTasksTab from '../../intelligence/IntelligenceTasksTab';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

/**
 * Settings → Developer Options → Tasks.
 *
 * Hosts the {@link IntelligenceTasksTab} task-board surface that previously
 * lived as a tab on the Activity page. The board (personal to-dos, task-source
 * boards, and the per-agent boards built across conversations) is unchanged —
 * this panel only re-homes it under the developer menu with the standard
 * SettingsHeader + breadcrumb chrome.
 */
const TasksPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  return (
    <div className="z-10 relative" data-testid="tasks-panel">
      <SettingsHeader
        title={t('memory.tab.tasks')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />

      <div className="p-4">
        <p className="mb-4 text-xs text-neutral-500 dark:text-neutral-400">
          {t('memory.tab.tasksDescription')}
        </p>
        <IntelligenceTasksTab />
      </div>
    </div>
  );
};

export default TasksPanel;
