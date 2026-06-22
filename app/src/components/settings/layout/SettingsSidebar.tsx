import { useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useSettingsNavigation } from '../hooks/useSettingsNavigation';
import SettingsSearchBar from '../search/SettingsSearchBar';
import { useSettingsSearch } from '../search/useSettingsSearch';
import {
  entryRoute,
  NAV_GROUP_LABEL_KEY,
  resolveSidebarId,
  sidebarGroups,
} from '../settingsRouteRegistry';
import { SETTINGS_NAV_ICONS } from './settingsNavIcons';

/** A renderable nav row, normalised across grouped entries and search results. */
interface NavRow {
  id: string;
  label: string;
  route: string;
  /** Accent the row (e.g. Billing) even when inactive. */
  highlight?: boolean;
}

interface NavSection {
  key: string;
  /** i18n key for the group heading, or null to render no heading (search results). */
  labelKey: string | null;
  rows: NavRow[];
}

/**
 * Grouped settings navigation. On wide viewports this is the persistent left
 * pane of the two-pane layout; on narrow viewports it doubles as the
 * /settings index page (the old drill-down home list).
 */
const SettingsSidebar = () => {
  const { t } = useT();
  const { currentRoute, navigateToSettings } = useSettingsNavigation();

  // While searching we render a flat, ranked result list backed by the FULL
  // route registry (via useSettingsSearch) — not just the top-level sidebar
  // entries — so deep/sub-nav destinations (privacy, security, agent-access, …)
  // remain reachable via search. With no query we render the grouped nav.
  const [searchQuery, setSearchQuery] = useState('');
  const isSearching = searchQuery.trim().length > 0;
  const searchResults = useSettingsSearch(searchQuery);

  const activeSidebarId = resolveSidebarId(currentRoute);
  const sections: NavSection[] = isSearching
    ? [
        {
          key: 'results',
          labelKey: null,
          rows: searchResults.map(result => ({
            id: result.entry.id,
            label: result.title,
            route: result.entry.route,
          })),
        },
      ]
    : sidebarGroups().map(group => ({
        key: group.group,
        labelKey: NAV_GROUP_LABEL_KEY[group.group],
        rows: group.entries.map(entry => ({
          id: entry.id,
          label: t(entry.titleKey),
          route: entryRoute(entry),
          highlight: entry.highlight,
        })),
      }));
  const hasRows = sections.some(section => section.rows.length > 0);

  return (
    <nav
      aria-label={t('nav.settings')}
      data-walkthrough="settings-menu"
      className="flex h-full flex-col">
      {/* Full-width search field as a fixed header (no padding). The scroll
          lives on the content below, not on this header. */}
      <SettingsSearchBar value={searchQuery} onValueChange={setSearchQuery} />

      <div className="min-h-0 flex-1 overflow-y-auto px-1.5 pb-2">
        {sections.map(section => (
          <div
            key={section.key}
            data-testid={
              section.labelKey ? `settings-sidebar-group-${section.key}` : 'settings-search-results'
            }>
            {section.labelKey && (
              <div className="px-2 pb-0.5 pt-2.5">
                <span className="text-[10px] font-semibold uppercase tracking-wider text-stone-500 dark:text-neutral-400">
                  {t(section.labelKey)}
                </span>
              </div>
            )}
            <ul>
              {section.rows.map(row => {
                const active = activeSidebarId === row.id;
                const highlight = !!row.highlight;
                const rowClass = active
                  ? // Active rows highlight both background and text in the accent colour.
                    'bg-primary-50 font-medium text-primary-700 dark:bg-primary-500/15 dark:text-primary-200'
                  : highlight
                    ? // Highlighted-but-inactive rows accent the text only (no bg).
                      'font-medium text-primary-700 hover:bg-stone-50 dark:text-primary-300 dark:hover:bg-neutral-800/60'
                    : 'text-stone-600 hover:bg-stone-50 hover:text-stone-900 dark:text-neutral-300 dark:hover:bg-neutral-800/60 dark:hover:text-neutral-100';
                return (
                  <li key={row.id}>
                    <button
                      type="button"
                      data-testid={`settings-nav-${row.id}`}
                      aria-current={active ? 'page' : undefined}
                      onClick={() => navigateToSettings(row.route)}
                      className={`flex w-full items-center gap-2 rounded-md px-2 py-1 text-left text-[13px] transition-colors ${rowClass}`}>
                      <span
                        className={`shrink-0 ${
                          active || highlight
                            ? 'text-primary-600 dark:text-primary-400'
                            : 'text-stone-400 dark:text-neutral-500'
                        }`}>
                        {SETTINGS_NAV_ICONS[row.id] ?? null}
                      </span>
                      <span className="truncate">{row.label}</span>
                    </button>
                  </li>
                );
              })}
            </ul>
          </div>
        ))}

        {isSearching && !hasRows && (
          <p
            data-testid="settings-search-empty"
            className="px-2 pt-3 text-center text-xs text-stone-400 dark:text-neutral-500">
            {t('settings.settingsSearch.noResults').replace('{query}', searchQuery.trim())}
          </p>
        )}
      </div>
    </nav>
  );
};

export default SettingsSidebar;
