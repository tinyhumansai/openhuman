import { Routes, Route } from 'react-router-dom';
import { useLocation } from 'react-router-dom';
import SettingsLayout from './SettingsLayout';
import SettingsHome from './SettingsHome';
import ConnectionsPanel from './panels/ConnectionsPanel';
import MessagingPanel from './panels/MessagingPanel';
import PrivacyPanel from './panels/PrivacyPanel';
import ProfilePanel from './panels/ProfilePanel';
import AdvancedPanel from './panels/AdvancedPanel';
import BillingPanel from './panels/BillingPanel';
import { useSettingsNavigation } from './hooks/useSettingsNavigation';

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
      </Routes>
    </SettingsLayout>
  );
};

export default SettingsModal;