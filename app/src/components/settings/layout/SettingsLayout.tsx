import debug from 'debug';
import { Outlet } from 'react-router-dom';

import TwoPanelLayout from '../../layout/TwoPanelLayout';
import { SettingsLayoutProvider } from './SettingsLayoutContext';
import SettingsSidebar from './SettingsSidebar';
import SettingsSubNav from './SettingsSubNav';

const log = debug('settings:layout');

/**
 * Two-pane settings shell, built on the reusable {@link TwoPanelLayout}.
 *
 * The grouped navigation sidebar is always shown and the layout spans the full
 * width of the page; the sidebar is resizable (drag the divider) and its width
 * persists per user via the `layout` slice (id `settings`). Each pane scrolls
 * independently, so the nav and the routed panel never fight over one
 * scrollbar.
 */
const SettingsLayout = () => {
  log('render');

  return (
    <SettingsLayoutProvider value={{ inTwoPaneShell: true }}>
      <TwoPanelLayout
        id="settings"
        // Max-width is applied once to the whole panel (sidebar + content
        // together) and centered, rather than capping each settings panel.
        // Card panes (bg / border / rounding) come from TwoPanelLayout's
        // default paneClassName, matching the conversations two-pane.
        className="mx-auto h-full w-full max-w-6xl p-4 pt-6"
        defaultSidebarVisible
        defaultSidebarWidth={288}
        minSidebarWidth={220}
        maxSidebarWidth={420}
        showDividerHandle={false}
        sidebar={
          // overflow-hidden so the scroll lives on the sidebar's own content
          // area (below the fixed search header), not this wrapper.
          <div className="h-full overflow-hidden">
            <SettingsSidebar />
          </div>
        }>
        <div className="h-full overflow-y-auto">
          <div className="px-4 pt-4 -mb-4">
            <SettingsSubNav />
          </div>
          <Outlet />
        </div>
      </TwoPanelLayout>
    </SettingsLayoutProvider>
  );
};

export default SettingsLayout;
