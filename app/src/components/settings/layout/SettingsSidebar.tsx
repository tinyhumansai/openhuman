import { useT } from '../../../lib/i18n/I18nContext';
import TwoPaneNav from '../../layout/TwoPaneNav';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import {
  entryRoute,
  NAV_GROUP_LABEL_KEY,
  resolveSidebarId,
  sidebarGroups,
} from '../settingsRouteRegistry';
import { SETTINGS_NAV_ICONS } from './settingsNavIcons';

/**
 * Grouped settings navigation. On wide viewports this is the persistent left
 * pane of the two-pane layout; on narrow viewports it doubles as the
 * /settings index page (the old drill-down home list).
 *
 * Rendered with the shared {@link TwoPaneNav} — the same primitive every other
 * page's sidebar uses, and the same column it is projected into. It used to
 * hand-roll an equivalent row list, which drifted the moment either side was
 * touched: shorter rows (`py-1` vs `py-1.5`), a different group-heading inset,
 * and a grey icon left sitting on the accent fill of a selected row.
 *
 * There is no search field here any more. It filtered against the full route
 * registry rather than these top-level entries, so it was the only way to reach
 * some deep destinations (privacy, security, agent-access, …) by name; those are
 * still reachable by navigating their parent section, but nothing types them
 * now. `components/settings/search/` went with it — that directory existed
 * solely for this header and had no other importer.
 */
const SettingsSidebar = () => {
  const { t } = useT();
  const { currentRoute, navigateToSettings } = useSettingsNavigation();

  const activeSidebarId = resolveSidebarId(currentRoute);

  // `route` is not carried on the nav item, so keep the id → route map beside
  // the groups and resolve on select.
  const routeById = new Map<string, string>();

  const groups = sidebarGroups().map(group => ({
    label: t(NAV_GROUP_LABEL_KEY[group.group]),
    testId: `settings-sidebar-group-${group.group}`,
    items: group.entries.map(entry => {
      routeById.set(entry.id, entryRoute(entry));
      return {
        value: entry.id,
        label: t(entry.titleKey),
        icon: SETTINGS_NAV_ICONS[entry.id] ?? null,
        highlight: entry.highlight,
        testId: `settings-nav-${entry.id}`,
      };
    }),
  }));

  return (
    <TwoPaneNav
      ariaLabel={t('nav.settings')}
      walkthroughId="settings-menu"
      groups={groups}
      selected={activeSidebarId ?? ''}
      onSelect={id => {
        const route = routeById.get(id);
        if (route) navigateToSettings(route);
      }}
    />
  );
};

export default SettingsSidebar;
