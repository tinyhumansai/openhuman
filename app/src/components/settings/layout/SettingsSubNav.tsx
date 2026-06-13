import { useT } from '../../../lib/i18n/I18nContext';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import { entryRoute, resolveSidebarId, subNavSiblings } from '../settingsRouteRegistry';

/**
 * Pill-tab row of real route links shown above panels that belong to a
 * sidebar family (e.g. Account → Team / Privacy / Security / Migration).
 * Each pill navigates to its own route — no nested hub pages.
 */
const SettingsSubNav = () => {
  const { t } = useT();
  const { currentRoute, navigateToSettings } = useSettingsNavigation();

  const sidebarId = resolveSidebarId(currentRoute);
  const siblings = sidebarId ? subNavSiblings(sidebarId) : [];

  if (siblings.length === 0) return null;

  return (
    <div
      className="flex flex-wrap gap-1.5 px-1 pb-3"
      role="navigation"
      aria-label={t('nav.settings')}
      data-testid="settings-subnav">
      {siblings.map(entry => {
        const active = entry.id === currentRoute;
        return (
          <button
            key={entry.id}
            type="button"
            data-testid={`settings-subnav-${entry.id}`}
            aria-current={active ? 'page' : undefined}
            onClick={() => navigateToSettings(entryRoute(entry))}
            className={`rounded-full px-3 py-1 text-xs font-medium transition-colors ${
              active
                ? 'bg-stone-800 text-white dark:bg-neutral-100 dark:text-neutral-900'
                : 'bg-white dark:bg-neutral-900 border border-stone-200 dark:border-neutral-800 text-stone-600 dark:text-neutral-300 hover:bg-stone-50 dark:hover:bg-neutral-800'
            }`}>
            {t(entry.titleKey)}
          </button>
        );
      })}
    </div>
  );
};

export default SettingsSubNav;
