import { Route, Routes } from 'react-router-dom';

import SettingsLayout from '../components/settings/layout/SettingsLayout';
import { settingsRouteElements } from '../components/settings/settingsRouteElements';

/**
 * Full-page Settings host, on every target. Wraps the shared
 * {@link settingsRouteElements} route table in {@link SettingsLayout}, which
 * projects the grouped settings nav into the app sidebar's dynamic region and
 * renders the routed panel as a card — the same shape as every other routed
 * page. Retired slugs are kept as redirects inside the shared table so deep
 * links keep working.
 *
 * Desktop presented this as a centered modal over a stashed background location
 * until that overlay was retired; `/settings/*` is now an ordinary route in
 * both `AppRoutes` and `AppRoutesIOS`.
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
