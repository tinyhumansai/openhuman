import { useT } from '../../../lib/i18n/I18nContext';
import IntelligenceTasksTab from '../../intelligence/IntelligenceTasksTab';
import SettingsPanel from '../layout/SettingsPanel';

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

  return (
    <SettingsPanel testId="tasks-panel" description={t('settings.developerMenu.tasks.desc')}>
      <>
        <p className="mb-4 text-xs text-content-muted">{t('memory.tab.tasksDescription')}</p>
        <IntelligenceTasksTab />
      </>
    </SettingsPanel>
  );
};

export default TasksPanel;
