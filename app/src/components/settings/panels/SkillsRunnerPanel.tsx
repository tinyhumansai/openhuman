// Settings → Developer Options → Skills Runner — thin wrapper around the
// reusable `<SkillsRunnerBody />` so the settings shell (header + back
// button + breadcrumbs) stays consistent with other panels. The actual
// picker / Run / Schedule / Recent Runs UX lives in
// `app/src/components/skills/SkillsRunnerBody.tsx`, shared with the
// top-level /skills page's "Runners" tab.
import { useT } from '../../../lib/i18n/I18nContext';
import SkillsRunnerBody from '../../skills/SkillsRunnerBody';
import SettingsHeader from '../components/SettingsHeader';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';

const SkillsRunnerPanel = () => {
  const { t } = useT();
  const { navigateBack, breadcrumbs } = useSettingsNavigation();

  return (
    <div className="flex flex-col h-full">
      <SettingsHeader
        title={t('settings.developerMenu.skillsRunner.title')}
        showBackButton={true}
        onBack={navigateBack}
        breadcrumbs={breadcrumbs}
      />
      <div className="flex-1 overflow-y-auto p-6">
        <SkillsRunnerBody />
      </div>
    </div>
  );
};

export default SkillsRunnerPanel;
