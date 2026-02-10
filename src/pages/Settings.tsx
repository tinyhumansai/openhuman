import { Route, Routes } from 'react-router-dom';

import AdvancedPanel from '../components/settings/panels/AdvancedPanel';
import BillingPanel from '../components/settings/panels/BillingPanel';
import ConnectionsPanel from '../components/settings/panels/ConnectionsPanel';
import MessagingPanel from '../components/settings/panels/MessagingPanel';
import PrivacyPanel from '../components/settings/panels/PrivacyPanel';
import ProfilePanel from '../components/settings/panels/ProfilePanel';
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
        <Route path="privacy" element={<PrivacyPanel />} />
        <Route path="profile" element={<ProfilePanel />} />
        <Route path="advanced" element={<AdvancedPanel />} />
        <Route path="billing" element={<BillingPanel />} />
        <Route path="team" element={<TeamPanel />} />
        <Route path="team/manage/:teamId" element={<TeamManagementPanel />} />
        <Route path="team/manage/:teamId/members" element={<TeamMembersPanel />} />
        <Route path="team/manage/:teamId/invites" element={<TeamInvitesPanel />} />
        <Route path="team/members" element={<TeamMembersPanel />} />
        <Route path="team/invites" element={<TeamInvitesPanel />} />
      </Routes>
    </div>
  );
};

export default Settings;
