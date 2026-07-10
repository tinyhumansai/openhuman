// Settings → Developer Options → Skills Runner — thin wrapper around the
// reusable `<WorkflowRunnerBody />` so the settings shell (header + back
// button + breadcrumbs) stays consistent with other panels. The actual
// picker / Run / Schedule / Recent Runs UX lives in
// `app/src/components/skills/WorkflowRunnerBody.tsx`, shared with the
// top-level /skills page's "Runners" tab.
import { useT } from '../../../lib/i18n/I18nContext';
import WorkflowRunnerBody from '../../skills/WorkflowRunnerBody';
import SettingsPanel from '../layout/SettingsPanel';

const WorkflowRunnerPanel = () => {
  const { t } = useT();

  return (
    <SettingsPanel description={t('settings.developerMenu.skillsRunner.desc')}>
      <WorkflowRunnerBody />
    </SettingsPanel>
  );
};

export default WorkflowRunnerPanel;
