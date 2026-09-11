import debug from 'debug';
import { Outlet } from 'react-router-dom';

import { SidebarContent } from '../../layout/shell/SidebarSlot';
import { SettingsLayoutProvider } from './SettingsLayoutContext';
import SettingsSidebar from './SettingsSidebar';

const log = debug('settings:layout');

/**
 * Settings shell, used by every target. The grouped navigation lives in the
 * root app sidebar's dynamic region (projected via {@link SidebarContent});
 * this component only renders the routed panel, which owns the single vertical
 * scroll. The sibling sub-nav chips are rendered inside each panel's header
 * (via SettingsPanel).
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
          and renders its own header (title, description, sibling sub-nav).

          It renders flush, NOT inside a card. The panel used to sit on its own
          `rounded-2xl border bg-surface shadow-soft` sheet, which put a second
          bordered container inside the shell's content surface — a card on a
          card, with the page's own header and full-bleed divider inside the
          inner one. The shell already provides the surface; every other routed
          page uses it directly, and settings does now too. */}
      {/* No `max-w-*`/`mx-auto` here: capping the column and centring it reads
          as a left/right gutter the other routed pages do not have, which is
          what made settings look inset. The pane is the width of the content
          surface, same as Connections or Workflows. */}
      <div className="flex h-full min-h-0 w-full flex-col">
        <div className="min-h-0 flex-1 overflow-hidden">
          <Outlet />
        </div>
      </div>
    </SettingsLayoutProvider>
  );
};

export default SettingsLayout;
