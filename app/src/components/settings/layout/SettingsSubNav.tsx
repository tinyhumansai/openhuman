import { useT } from '../../../lib/i18n/I18nContext';
import ChipTabs, { type ChipTabItem } from '../../layout/ChipTabs';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import { entryRoute, resolveSidebarId, subNavSiblings } from '../settingsRouteRegistry';

interface SettingsSubNavProps {
  /**
   * Row container classes. Defaults to the standalone spacing used when the row
   * sits on its own; the {@link SettingsPanel} header passes a tighter value so
   * the pills tuck directly under the title/description.
   */
  className?: string;
}

/**
 * Pill-tab row of real route links shown for panels that belong to a sidebar
 * family (e.g. Account → Team / Privacy / Security / Migration). Each pill
 * navigates to its own route — no nested hub pages. Rendered with the shared
 * {@link ChipTabs} bar (nav semantics) so it matches every other chip row in
 * the app. Returns `null` when the active route has no siblings.
 */
const SettingsSubNav = ({
  className = 'flex flex-wrap gap-1.5 px-4 pt-4 pb-3',
}: SettingsSubNavProps = {}) => {
  const { t } = useT();
  const { currentRoute, navigateToSettings } = useSettingsNavigation();

  const sidebarId = resolveSidebarId(currentRoute);
  const siblings = sidebarId ? subNavSiblings(sidebarId) : [];

  if (siblings.length === 0) return null;

  const items: ChipTabItem<string>[] = siblings.map(entry => ({
    id: entry.id,
    label: t(entry.titleKey),
    testId: `settings-subnav-${entry.id}`,
  }));

  // The siblings always include the current route's own entry; fall back to the
  // first chip so the bar still renders if it somehow doesn't.
  const value = siblings.some(s => s.id === currentRoute) ? currentRoute : siblings[0].id;

  return (
    <ChipTabs
      as="nav"
      ariaLabel={t('nav.settings')}
      testId="settings-subnav"
      className={className}
      items={items}
      value={value}
      onChange={id => {
        const entry = siblings.find(s => s.id === id);
        if (entry) navigateToSettings(entryRoute(entry));
      }}
    />
  );
};

export default SettingsSubNav;
