import { useT } from '../../../lib/i18n/I18nContext';
import IntelligenceTasksTab from '../../intelligence/IntelligenceTasksTab';
import PanelPage from '../../layout/PanelPage';
import SettingsBackButton from '../components/SettingsBackButton';
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
  const { navigateBack } = useSettingsNavigation();

  return (
    <PanelPage
      className="z-10"
      contentClassName=""
      testId="tasks-panel"
      description={t('settings.developerMenu.tasks.desc')}
      leading={<SettingsBackButton onBack={navigateBack} />}>
      <div className="p-4">
        <p className="mb-4 text-xs text-neutral-500 dark:text-neutral-400">
          {t('memory.tab.tasksDescription')}
        </p>
        <IntelligenceTasksTab />
      </div>
    </PanelPage>
  );
};

export default TasksPanel;
