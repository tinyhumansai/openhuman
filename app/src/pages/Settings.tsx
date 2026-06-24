import { Route, Routes } from 'react-router-dom';

import SettingsLayout from '../components/settings/layout/SettingsLayout';
import { settingsRouteElements } from '../components/settings/settingsRouteElements';

/**
 * Full-page Settings host. Used on iOS (and any non-desktop target) where
 * Settings is a routed screen rather than the desktop modal overlay. Wraps the
 * shared {@link settingsRouteElements} route table in the two-pane
 * {@link SettingsLayout} (persistent sidebar on md+, drill-down on narrow
 * viewports). Retired slugs are kept as redirects inside the shared table so
 * deep links keep working.
 *
 * Desktop no longer mounts this — it renders the same route table inside
 * `SettingsModal` (see components/settings/modal/). The route elements are
 * shared so both hosts stay in lockstep.
 */
const Settings = () => {
  return (
    // h-full chains the AppShell page-scroller height down to SettingsLayout so
    // its panes can bound to the viewport and scroll internally.
    <div className="h-full">
      <Routes>
        <Route element={<SettingsLayout />}>{settingsRouteElements()}</Route>
      </Routes>
    </div>
  );
};

export default Settings;
