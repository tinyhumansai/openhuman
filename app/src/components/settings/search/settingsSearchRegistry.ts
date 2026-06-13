// ---------------------------------------------------------------------------
// Settings search registry
//
// Derived from settingsRouteRegistry so that every destination registered
// there automatically becomes searchable. The mapping adds:
//  - `sectionKey`: the i18n key for the section badge shown in results.
//  - `keywords`:   English-only synonyms (from the registry's searchKeywords).
//
// The Phase 1 "shallow" search API surface is preserved: SETTINGS_SEARCH_ENTRIES
// and SettingsSearchEntry remain the public interface consumed by
// useSettingsSearch / SettingsSearchBar. A future Phase 2 can enrich this
// with per-control deep search without changing the consumer API.
//
// `devOnly` entries are only surfaced when developer mode is on.
// Routes map 1:1 to the <Route> table in app/src/pages/Settings.tsx.
// ---------------------------------------------------------------------------
import debug from 'debug';

import {
  entryRoute,
  SETTINGS_ROUTE_REGISTRY,
  type SettingsSection,
} from '../settingsRouteRegistry';

const log = debug('settings:search');

export interface SettingsSearchEntry {
  /** Stable unique id — used as the React key and test id. */
  id: string;
  /** i18n key for the result title (reused from the existing menu item). */
  titleKey: string;
  /** i18n key for the result description (optional). */
  descriptionKey?: string;
  /** Settings route passed to `navigateToSettings(route)`. */
  route: string;
  /** i18n key for the section badge shown next to the result. */
  sectionKey: string;
  /** Extra English match terms (synonyms). Not shown in the UI. */
  keywords?: string[];
  /** When true, only surfaced if developer mode is enabled. */
  devOnly?: boolean;
}

// Section badge i18n keys (reused from the existing section headers).
const SECTION_KEY: Record<SettingsSection, string> = {
  home: 'nav.settings',
  account: 'settings.groups.account',
  ai: 'pages.settings.aiSection.title',
  agents: 'settings.agentsSection.title',
  features: 'pages.settings.featuresSection.title',
  crypto: 'settings.cryptoSection.title',
  notifications: 'settings.groups.notifications',
  developer: 'settings.developerDiagnostics',
};

// Fine-grained badge overrides: top-level 'home' entries map to 'nav.settings'
// by default, but some items logically belong to a sub-group badge.
const SECTION_BADGE_OVERRIDES: Record<string, string> = {
  personality: 'settings.groups.assistant',
  appearance: 'settings.groups.account',
  devices: 'settings.groups.account',
  'memory-sync': 'settings.groups.account',
  'notifications-hub': 'settings.groups.notifications',
  crypto: 'settings.cryptoSection.title',
  about: 'settings.about',
  ai: 'nav.settings',
  'agents-settings': 'nav.settings',
  features: 'nav.settings',
  integrations: 'nav.settings',
  'developer-options': 'settings.developerDiagnostics',
};

/**
 * Every searchable settings destination. Derived from the route registry so
 * additions to the registry automatically appear in search. Hidden deep-link
 * entries are excluded from search results.
 */
export const SETTINGS_SEARCH_ENTRIES: SettingsSearchEntry[] = SETTINGS_ROUTE_REGISTRY.filter(
  e => !e.hiddenDeepLink
).map(entry => {
  const route = entryRoute(entry);
  const sectionKey = SECTION_BADGE_OVERRIDES[entry.id] ?? SECTION_KEY[entry.section];

  return {
    id: entry.id,
    titleKey: entry.titleKey,
    descriptionKey: entry.descriptionKey,
    route,
    sectionKey,
    keywords: entry.searchKeywords,
    devOnly: entry.devOnly,
  };
});

// Debug log: confirm derived entries.
if (typeof window !== 'undefined') {
  log('search registry derived — %d entries', SETTINGS_SEARCH_ENTRIES.length);
}
