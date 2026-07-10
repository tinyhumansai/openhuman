import debug from 'debug';
import { Outlet } from 'react-router-dom';

import { SidebarContent } from '../../layout/shell/SidebarSlot';
import { SettingsLayoutProvider } from './SettingsLayoutContext';
import SettingsSidebar from './SettingsSidebar';

const log = debug('settings:layout');

/**
 * Settings shell. The grouped navigation now lives in the root app sidebar's
 * dynamic region (projected via {@link SidebarContent}); this component only
 * renders the routed panel, which owns the single vertical scroll. The sibling
 * sub-nav chips are rendered inside each panel's header (via SettingsPanel).
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
      {/* Bounded flex column: the routed panel owns the only vertical scroll
          (its WrappedSettingsPage / PanelScaffold) and renders its own header
          (title, description, sibling sub-nav). The panel is wrapped in a card
          so settings pages get a surface/background instead of sitting flush. */}
      <div className="mx-auto flex h-full min-h-0 w-full max-w-5xl flex-col p-4">
        <div className="min-h-0 flex-1 overflow-hidden rounded-2xl border border-line bg-surface shadow-soft">
          <Outlet />
        </div>
      </div>
    </SettingsLayoutProvider>
  );
};

export default SettingsLayout;
