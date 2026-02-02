import { Route, Routes, useLocation } from 'react-router-dom';

import { useSettingsNavigation } from './hooks/useSettingsNavigation';
import AdvancedPanel from './panels/AdvancedPanel';
import BillingPanel from './panels/BillingPanel';
import ConnectionsPanel from './panels/ConnectionsPanel';
import MessagingPanel from './panels/MessagingPanel';
import PrivacyPanel from './panels/PrivacyPanel';
import ProfilePanel from './panels/ProfilePanel';
import TeamInvitesPanel from './panels/TeamInvitesPanel';
import TeamMembersPanel from './panels/TeamMembersPanel';
import TeamPanel from './panels/TeamPanel';
import SettingsHome from './SettingsHome';
import SettingsLayout from './SettingsLayout';

const SettingsModal = () => {
  const location = useLocation();
  const { closeSettings } = useSettingsNavigation();

  // Only render modal when on settings routes
  const isSettingsRoute = location.pathname.startsWith('/settings');

  if (!isSettingsRoute) {
    return null;
  }

  return (
    <SettingsLayout onClose={closeSettings}>
      <Routes>
        <Route path="/settings" element={<SettingsHome />} />
        <Route path="/settings/connections" element={<ConnectionsPanel />} />
        <Route path="/settings/messaging" element={<MessagingPanel />} />
        <Route path="/settings/privacy" element={<PrivacyPanel />} />
        <Route path="/settings/profile" element={<ProfilePanel />} />
        <Route path="/settings/advanced" element={<AdvancedPanel />} />
        <Route path="/settings/billing" element={<BillingPanel />} />
        <Route path="/settings/team" element={<TeamPanel />} />
        <Route path="/settings/team/members" element={<TeamMembersPanel />} />
        <Route path="/settings/team/invites" element={<TeamInvitesPanel />} />
      </Routes>
    </SettingsLayout>
  );
};

export default SettingsModal;
