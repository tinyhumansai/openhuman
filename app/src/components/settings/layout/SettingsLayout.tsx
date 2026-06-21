import debug from 'debug';
import { Outlet } from 'react-router-dom';

import { SidebarContent } from '../../layout/shell/SidebarSlot';
import { SettingsLayoutProvider } from './SettingsLayoutContext';
import SettingsSidebar from './SettingsSidebar';
import SettingsSubNav from './SettingsSubNav';

const log = debug('settings:layout');

/**
 * Settings shell. The grouped navigation now lives in the root app sidebar's
 * dynamic region (projected via {@link SidebarContent}); this component only
 * renders the routed panel — the sub-nav chips pinned at top and the routed
 * page owning the single vertical scroll below.
 */
const SettingsLayout = () => {
  log('render');

  return (
    <SettingsLayoutProvider value={{ inTwoPaneShell: true }}>
      <SidebarContent>
        <div className="h-full overflow-hidden">
          <SettingsSidebar />
        </div>
      </SidebarContent>
      {/* Bounded flex column: the sub-nav chips stay pinned at the top while
          the routed panel owns the only vertical scroll (its WrappedSettingsPage
          / PanelScaffold). The panel is wrapped in a card so settings pages get
          a surface/background instead of sitting flush on the shell. */}
      <div className="mx-auto flex h-full min-h-0 w-full max-w-5xl flex-col gap-3 p-4">
        <div className="flex-shrink-0">
          <SettingsSubNav />
        </div>
        <div className="min-h-0 flex-1 overflow-hidden rounded-2xl border border-stone-200 bg-white shadow-soft dark:border-neutral-800 dark:bg-neutral-900">
          <Outlet />
        </div>
      </div>
    </SettingsLayoutProvider>
  );
};

export default SettingsLayout;
