import { Route, Routes } from 'react-router-dom';

import AccessibilityPanel from '../components/settings/panels/AccessibilityPanel';
import AdvancedPanel from '../components/settings/panels/AdvancedPanel';
import AgentChatPanel from '../components/settings/panels/AgentChatPanel';
import AIPanel from '../components/settings/panels/AIPanel';
import AutocompletePanel from '../components/settings/panels/AutocompletePanel';
import BillingPanel from '../components/settings/panels/BillingPanel';
import ConnectionsPanel from '../components/settings/panels/ConnectionsPanel';
import CronJobsPanel from '../components/settings/panels/CronJobsPanel';
import DeveloperOptionsPanel from '../components/settings/panels/DeveloperOptionsPanel';
import LocalModelPanel from '../components/settings/panels/LocalModelPanel';
import MemoryDebugPanel from '../components/settings/panels/MemoryDebugPanel';
import MessagingPanel from '../components/settings/panels/MessagingPanel';
import PrivacyPanel from '../components/settings/panels/PrivacyPanel';
import ProfilePanel from '../components/settings/panels/ProfilePanel';
import ScreenIntelligencePanel from '../components/settings/panels/ScreenIntelligencePanel';
import SkillsPanel from '../components/settings/panels/SkillsPanel';
import TauriCommandsPanel from '../components/settings/panels/TauriCommandsPanel';
import TeamInvitesPanel from '../components/settings/panels/TeamInvitesPanel';
import TeamManagementPanel from '../components/settings/panels/TeamManagementPanel';
import TeamMembersPanel from '../components/settings/panels/TeamMembersPanel';
import TeamPanel from '../components/settings/panels/TeamPanel';
import SettingsHome from '../components/settings/SettingsHome';

const Settings = () => {
  return (
    <div className="h-full overflow-hidden flex flex-col z-10 relative">
      <Routes>
        <Route index element={<SettingsHome />} />
        <Route path="connections" element={<ConnectionsPanel />} />
        <Route path="messaging" element={<MessagingPanel />} />
        <Route path="cron-jobs" element={<CronJobsPanel />} />
        <Route path="screen-intelligence" element={<ScreenIntelligencePanel />} />
        <Route path="autocomplete" element={<AutocompletePanel />} />
        <Route path="privacy" element={<PrivacyPanel />} />
        <Route path="profile" element={<ProfilePanel />} />
        <Route path="advanced" element={<AdvancedPanel />} />
        <Route path="agent-chat" element={<AgentChatPanel />} />
        <Route path="ai" element={<AIPanel />} />
        <Route path="accessibility" element={<AccessibilityPanel />} />
        <Route path="local-model" element={<LocalModelPanel />} />
        <Route path="billing" element={<BillingPanel />} />
        <Route path="skills" element={<SkillsPanel />} />
        <Route path="team" element={<TeamPanel />} />
        <Route path="team/manage/:teamId" element={<TeamManagementPanel />} />
        <Route path="team/manage/:teamId/members" element={<TeamMembersPanel />} />
        <Route path="team/manage/:teamId/invites" element={<TeamInvitesPanel />} />
        <Route path="team/members" element={<TeamMembersPanel />} />
        <Route path="team/invites" element={<TeamInvitesPanel />} />
        <Route path="developer-options" element={<DeveloperOptionsPanel />} />
        <Route path="tauri-commands" element={<TauriCommandsPanel />} />
        <Route path="memory-debug" element={<MemoryDebugPanel />} />
      </Routes>
    </div>
  );
};

export default Settings;
