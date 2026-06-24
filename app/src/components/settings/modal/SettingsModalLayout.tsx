import { Outlet, Route, Routes } from 'react-router-dom';

import { SettingsLayoutProvider } from '../layout/SettingsLayoutContext';
import SettingsSidebar from '../layout/SettingsSidebar';
import SettingsSubNav from '../layout/SettingsSubNav';
import { settingsRouteElements } from '../settingsRouteElements';

/**
 * Two-column body of the desktop Settings modal: the grouped nav + search on the
 * left and the routed panel on the right, both inside the modal card.
 *
 * Unlike {@link SettingsLayout} (the iOS full-page host, which projects the
 * sidebar into the app shell via `SidebarContent`), this renders the sidebar
 * inline. It still advertises `inTwoPaneShell: true` so shared chrome
 * (SettingsHeader / SettingsBackButton) behaves exactly as in the full-page
 * shell — top-level panels hide their back button and rely on the sidebar.
 *
 * The shared {@link settingsRouteElements} use paths relative to `/settings`
 * (e.g. `account`). On iOS they are nested under the `/settings/*` route in
 * `AppRoutesIOS`; here there is no such ancestor (the modal is mounted directly
 * by the shell), so we scope them under an explicit `/settings` parent route
 * whose `<Outlet/>` renders the matched panel into the right column.
 */
export default function SettingsModalLayout() {
  return (
    <SettingsLayoutProvider value={{ inTwoPaneShell: true }}>
      <div className="flex h-full w-full min-h-0">
        <div className="h-full w-64 flex-shrink-0 overflow-hidden border-r border-stone-200 dark:border-neutral-800">
          <SettingsSidebar />
        </div>
        {/* Right column: the sub-nav pill row (Account → Team/Privacy/… ) pinned
            above the routed panel, mirroring the iOS full-page SettingsLayout. */}
        <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
          <div className="flex-shrink-0">
            <SettingsSubNav />
          </div>
          <div className="min-h-0 flex-1 overflow-hidden">
            <Routes>
              <Route path="/settings" element={<Outlet />}>
                {settingsRouteElements()}
              </Route>
            </Routes>
          </div>
        </div>
      </div>
    </SettingsLayoutProvider>
  );
}
