import debug from 'debug';

// ---------------------------------------------------------------------------
// Settings Route Registry
//
// Single declarative source of truth for every navigable settings destination.
// Consumers (SettingsHome, Settings.tsx section arrays, DeveloperOptionsPanel)
// derive their menus from here so that a route added once automatically appears
// in navigation and breadcrumbs. `settingsSearchRegistry` was a consumer too,
// until the settings sidebar's search field was removed and the whole
// `components/settings/search/` directory went with it.
//
// Section values determine the canonical breadcrumb parent:
//   'home'      → top-level home menu entry (Settings breadcrumb only)
//   'account'   → Settings → Account
//   'ai'        → Settings → AI & Models
//   'features'  → Settings → Features
//   'crypto'    → Settings → Crypto
//   'developer' → Settings → Developer & Diagnostics (devOnly entries)
//
// debug logging: [settings] registry loaded N entries
// ---------------------------------------------------------------------------

export type SettingsSection =
  | 'home'
  | 'account'
  | 'ai'
  | 'agents'
  | 'features'
  | 'crypto'
  | 'developer';

/**
 * Sidebar groups for the two-pane settings layout, in display order. The former
 * "System" group's Developer & Diagnostics sub-sections are now first-class
 * top-level groups.
 */
type SettingsNavGroup =
  | 'general'
  | 'appearance'
  | 'security'
  | 'data'
  | 'knowledgeMemory'
  | 'agentsAutonomy'
  | 'automationIntegrations'
  | 'diagnosticsLogs';

const NAV_GROUP_ORDER: SettingsNavGroup[] = [
  'general',
  'appearance',
  // Everything agentic in one category: agents, their tools, approvals and the
  // skills runner. Was split across Assistant, Connections and Agents & Autonomy.
  'agentsAutonomy',
  // Everything about what the assistant may touch: the credential store,
  // agent OS access (tiers, approvals, rate limits) and sandboxing.
  'security',
  'data',
  'knowledgeMemory',
  'automationIntegrations',
  'diagnosticsLogs',
];

/** i18n keys for the sidebar group labels. */
export const NAV_GROUP_LABEL_KEY: Record<SettingsNavGroup, string> = {
  general: 'settings.navGroups.general',
  appearance: 'settings.navGroups.appearance',
  // Reuses the Security page's title key, which is already translated.
  security: 'pages.settings.account.security',
  data: 'settings.navGroups.data',
  // Promoted from the old Developer & Diagnostics sub-sections.
  knowledgeMemory: 'settings.devGroups.knowledgeMemory',
  agentsAutonomy: 'settings.devGroups.agentsAutonomy',
  automationIntegrations: 'settings.devGroups.automationIntegrations',
  diagnosticsLogs: 'settings.devGroups.diagnosticsLogs',
};

interface SettingsRegistryEntry {
  /** Stable unique id — used as the React key, test id, and route slug. */
  id: string;
  /** Route segment passed to `navigateToSettings(id)` (defaults to `id`). */
  route?: string;
  /** i18n key for the entry title. */
  titleKey: string;
  /** i18n key for the entry description (optional). */
  descriptionKey?: string;
  /**
   * Canonical parent section. Determines:
   *  - Which home-group the entry appears in (for 'home' entries).
   *  - Which section-page items array the entry belongs to (for leaf panels).
   *  - The breadcrumb trail (Settings > <section-label> > <panel>).
   */
  section: SettingsSection;
  /**
   * When true the entry is only surfaced when developer mode is active.
   * These entries live under Settings → Developer & Diagnostics.
   */
  devOnly?: boolean;
  /** Extra English match terms (synonyms). Used by the search registry. */
  searchKeywords?: string[];
  /**
   * When true the route is intentionally hidden — accessible only via deep-link
   * or programmatic navigation. Not surfaced in any menu.
   */
  hiddenDeepLink?: boolean;
  /**
   * Sidebar group for the two-pane layout. Presence makes this entry a
   * top-level sidebar destination.
   */
  navGroup?: SettingsNavGroup;
  /**
   * Visually emphasise this sidebar entry (e.g. billing/upgrade) with an accent
   * colour so it stands out from the regular nav rows.
   */
  highlight?: boolean;
  /** Sort order within the sidebar group (ascending; defaults to 0). */
  navOrder?: number;
  /**
   * Id of the sidebar entry this route belongs to. Drives sidebar active-state
   * highlighting and the sub-nav pill row shown above the panel.
   */
  navParent?: string;
}

const log = debug('settings:registry');

// ---------------------------------------------------------------------------
// Registry entries
// ---------------------------------------------------------------------------

/**
 * Complete ordered list of every settings destination.
 *
 * Ordering within each section matches the target navigation tree. Items whose
 * `section` is 'home' are top-level home menu entries (the section-page hubs).
 * All other items are leaf panels belonging to the named section.
 */
export const SETTINGS_ROUTE_REGISTRY: SettingsRegistryEntry[] = [
  // =========================================================================
  // HOME — top-level section hubs shown on SettingsHome
  // =========================================================================

  // --- Account group (section hub) ---
  {
    id: 'account',
    titleKey: 'pages.settings.accountSection.title',
    descriptionKey: 'pages.settings.accountSection.description',
    section: 'home',
    searchKeywords: [
      'language',
      'locale',
      'translation',
      'profile',
      'sign out',
      'logout',
      'billing',
      'subscription',
      'payment',
      'plan',
      'invoice',
    ],
    navGroup: 'general',
    navOrder: 0,
  },
  {
    // appearance hosts the display-language selector (formerly an inline row on
    // the old settings home list) and the whole former Theme studio page —
    // palette, fonts, backdrop, import/export. `/settings/theme` redirects here.
    id: 'appearance',
    titleKey: 'settings.appearance.title',
    descriptionKey: 'settings.appearance.menuDesc',
    section: 'home',
    searchKeywords: [
      'theme',
      'dark',
      'light',
      'mode',
      'color',
      'colour',
      'font',
      'palette',
      'background',
      'backdrop',
    ],
    navGroup: 'appearance',
    navOrder: 0,
  },
  {
    // theme: Theme Studio — per-token colours, fonts, background and theme
    // import/export for the active theme. Was a tab of Appearance; the
    // Appearance gallery's "Custom" tile links here.
    id: 'theme',
    titleKey: 'settings.theme.title',
    descriptionKey: 'settings.theme.menuDesc',
    section: 'home',
    searchKeywords: [
      'theme studio',
      'custom theme',
      'palette',
      'colour',
      'color',
      'font',
      'import',
    ],
    navGroup: 'appearance',
    navOrder: 1,
  },
  // language: now a card on Account; devices: pairing page removed. Both
  // slugs redirect to /settings/account.

  // --- Assistant group ---
  // The old 'ai' and 'agents-settings' hub pages are retired — their slugs
  // redirect to /settings/llm and the Connections → Tools tab.
  {
    // personality and face: how the assistant presents itself, so they sit in
    // the Appearance group next to the app's own look. They were one page with
    // two tabs; `/settings/persona` redirects to personality, and
    // `/settings/mascot` and the old `personality#face` link to face.
    id: 'personality',
    titleKey: 'settings.assistant.personality',
    descriptionKey: 'settings.personality.menuDesc',
    section: 'home',
    searchKeywords: ['personality', 'tone', 'character', 'persona', 'name'],
    navGroup: 'appearance',
    navOrder: 2,
  },
  {
    id: 'face',
    titleKey: 'settings.face.title',
    descriptionKey: 'settings.face.menuDesc',
    section: 'home',
    searchKeywords: ['face', 'avatar', 'mascot', 'tiny', 'colour', 'color'],
    navGroup: 'appearance',
    navOrder: 3,
  },

  // --- Connections group ---
  // The Integrations settings section was retired — the composio/OAuth grid
  // lives on the Connections page and the task-source/webhook triage surface is
  // no longer used. Desktop Agent moved to the
  // Connections page's Desktop group; their slugs redirect there.

  // Notifications-hub and crypto hub pages are retired — their slugs redirect
  // to /settings/account and /settings/wallet-balances.

  // --- About ---
  {
    // Core connection — promotes cloud-mode remote-core config (persisted
    // RPC URL + token) into a first-class setting plus a live status
    // indicator (GH-4396). Sits just above About in General.
    id: 'core',
    titleKey: 'settings.core.title',
    descriptionKey: 'settings.core.menuDesc',
    section: 'home',
    searchKeywords: [
      'gateway',
      'core',
      'remote',
      'rpc',
      'url',
      'token',
      'cloud',
      'local',
      'connection',
      'server',
      'attach',
      'self-hosted',
    ],
    navGroup: 'general',
    navOrder: 97,
  },
  {
    // Moved out of its own top-level `/feedback` route: it was reached only
    // from a sidebar-header icon, and that icon became the command-palette
    // trigger. A public board is a General-settings subject anyway, next to
    // About.
    id: 'feedback',
    titleKey: 'nav.feedback',
    descriptionKey: 'feedback.header.desc',
    section: 'home',
    searchKeywords: ['feedback', 'bug', 'feature', 'request', 'board', 'vote'],
    navGroup: 'general',
    navOrder: 98,
  },
  {
    id: 'about',
    titleKey: 'settings.about',
    descriptionKey: 'settings.aboutDesc',
    section: 'home',
    searchKeywords: ['version', 'build', 'update', 'developer mode'],
    // Moved out of the retired "System" group; sits at the end of General.
    navGroup: 'general',
    navOrder: 99,
  },

  // =========================================================================
  // ACCOUNT section leaf panels
  // =========================================================================
  // Teams were removed from the product; the `team` entry went with them. The
  // route slugs survive as redirects in `settingsRouteElements`.
  //
  // Privacy, Security and Migration are their OWN sidebar rows rather than
  // sub-nav pills under Account (`navParent: 'account'`, as they were): each is
  // a full page of unrelated controls, and burying three of them behind one
  // Account row meant the sidebar named one destination for four pages.
  {
    id: 'privacy',
    titleKey: 'pages.settings.account.privacy',
    descriptionKey: 'pages.settings.account.privacyDesc',
    section: 'account',
    searchKeywords: ['telemetry', 'tracking', 'analytics', 'data'],
    navGroup: 'general',
    navOrder: 5,
  },
  {
    // Titled "Keychain": the page is secret storage and keychain status, and
    // "Security" is now the name of the category it sits in.
    id: 'security',
    titleKey: 'settings.keychain.title',
    descriptionKey: 'pages.settings.account.securityDesc',
    section: 'account',
    searchKeywords: ['keychain', 'secret', 'password', 'encryption', 'credentials', 'security'],
    navGroup: 'security',
    navOrder: 0,
  },
  {
    id: 'migration',
    titleKey: 'pages.settings.account.migration',
    descriptionKey: 'pages.settings.account.migrationDesc',
    section: 'account',
    searchKeywords: ['import', 'export', 'transfer', 'data'],
    navGroup: 'general',
    navOrder: 7,
  },

  // =========================================================================
  // AI section leaf panels
  // =========================================================================
  {
    id: 'llm',
    titleKey: 'pages.settings.ai.llm',
    descriptionKey: 'pages.settings.ai.llmDesc',
    section: 'ai',
    searchKeywords: ['model', 'anthropic', 'openai', 'claude', 'provider', 'api key'],
    // Surfaced on the Connections page (Intelligence group); route kept for
    // deep-link compatibility but no longer in the settings sidebar.
  },
  {
    id: 'embeddings',
    titleKey: 'pages.settings.ai.embeddings',
    descriptionKey: 'pages.settings.ai.embeddingsDesc',
    section: 'ai',
    searchKeywords: ['vector', 'embedding', 'search'],
    navParent: 'llm',
  },
  {
    id: 'voice',
    titleKey: 'pages.settings.ai.voice',
    descriptionKey: 'pages.settings.ai.voiceDesc',
    section: 'ai',
    searchKeywords: ['tts', 'stt', 'speech', 'dictation', 'audio'],
    // Surfaced on the Connections page (Intelligence group); route kept for
    // deep-link compatibility but no longer in the settings sidebar.
  },
  {
    // usage: merged Usage & Limits surface — cost dashboard, Tokenjuice token
    // savings (formerly the standalone token-usage page), and background loops
    // (formerly ledger-usage). Surfaced on the Connections page
    // (API-keys group); the route redirects there and it's no longer in the
    // settings sidebar. Legacy ledger-usage / cost-dashboard /
    // token-usage slugs redirect here.
    id: 'usage',
    titleKey: 'settings.usage.title',
    descriptionKey: 'settings.usage.menuDesc',
    section: 'ai',
    searchKeywords: [
      'usage',
      'tokens',
      'tokenjuice',
      'savings',
      'ledger',
      'cost',
      'spend',
      'loops',
      'background',
    ],
  },

  // =========================================================================
  // AGENTS section leaf panels
  // =========================================================================
  {
    // agent-access also hosts the autonomy rate-limit section (formerly the
    // standalone /settings/autonomy page — that slug redirects here).
    id: 'agent-access',
    titleKey: 'settings.agentAccess.title',
    descriptionKey: 'settings.agentAccess.menuDesc',
    section: 'agents',
    searchKeywords: [
      'access',
      'permissions',
      'tier',
      'security policy',
      'autonomy',
      'autonomous',
      'rate limit',
      'actions per hour',
      'auto-approve',
      'auto approve',
      'full autonomy',
      'bypass approval',
    ],
    // Was a sub-nav pill under Agents; now a page of the Security category.
    navGroup: 'security',
    navOrder: 1,
  },
  {
    id: 'sandbox-settings',
    titleKey: 'settings.sandbox.title',
    descriptionKey: 'settings.sandbox.menuDesc',
    section: 'agents',
    searchKeywords: ['sandbox', 'jail', 'isolation', 'docker'],
    navGroup: 'security',
    navOrder: 2,
  },

  // =========================================================================
  // FEATURES section leaf panels
  // =========================================================================
  {
    // meetings: Meeting Assistant settings (issue #3511 / epic #3505 PR-5).
    // Surfaced on the Connections page (meetings tab, below the meetings list);
    // the route redirects there and it's no longer in the settings sidebar.
    id: 'meetings',
    titleKey: 'settings.meetings.title',
    descriptionKey: 'settings.meetings.menuDesc',
    section: 'features',
    searchKeywords: [
      'meeting',
      'meet',
      'google meet',
      'auto join',
      'auto-join',
      'summarize',
      'summary',
      'listen only',
      'transcript',
    ],
  },

  // The Notifications settings page (preferences toggles) was removed
  // entirely; `/settings/notifications` now redirects to Account
  // (`settingsRouteElements`). Alerts remain reachable as the external
  // `/notifications` notification-center page, handled inline in Settings.tsx.

  // =========================================================================
  // CRYPTO section leaf panels
  // =========================================================================
  {
    id: 'recovery-phrase',
    titleKey: 'pages.settings.account.recoveryPhrase',
    descriptionKey: 'pages.settings.account.recoveryPhraseDesc',
    section: 'crypto',
    searchKeywords: ['mnemonic', 'seed', 'backup', 'recovery', 'wallet'],
    navParent: 'wallet-balances',
  },
  {
    // Surfaced on the Connections page (Integrations group); route redirects
    // there. Entry kept for search + deep-link compatibility.
    id: 'wallet-balances',
    titleKey: 'pages.settings.account.walletBalances',
    descriptionKey: 'pages.settings.account.walletBalancesDesc',
    section: 'crypto',
    searchKeywords: ['wallet', 'balance', 'tokens', 'crypto'],
  },

  // =========================================================================
  // DEVELOPER — debug-only entries (devOnly === true)
  // These live ONLY under Settings → Developer & Diagnostics.
  // Items removed from this list compared to the old DeveloperOptionsPanel:
  //   agents, autonomy, agent-access, sandbox-settings, activity-level,
  //   tools, voice, embeddings,
  //   ledger-usage, cost-dashboard, task-sources, composio-routing,
  //   webhooks-triggers, migration, security
  //   (all moved to their canonical section pages).
  // =========================================================================
  {
    // developer-options is the legacy aggregator panel — kept routable for deep
    // links, but no longer a sidebar entry now that its children are expanded
    // directly into the Developer & Diagnostics group.
    id: 'developer-options',
    titleKey: 'settings.developerDiagnostics',
    descriptionKey: 'settings.developerDiagnosticsDesc',
    section: 'home',
    devOnly: true,
    searchKeywords: ['developer', 'diagnostics', 'debug'],
  },
  // memory-engine / memory-data / memory-debug are redirects to the Memory
  // page's chips (Connections → Memory); they have no settings panel.
  // Knowledge & Memory group retired entirely — memory surfaces live on the
  // Memory page (engine / ask / learnings / conversations / documents / context).
  // voice-debug retired from the settings UI.
  {
    id: 'event-log',
    titleKey: 'settings.developerMenu.eventLog.title',
    descriptionKey: 'settings.developerMenu.eventLog.desc',
    section: 'developer',
    devOnly: true,
    navGroup: 'diagnosticsLogs',
    searchKeywords: ['events', 'log'],
  },
  {
    // Diagnostics lives under Events & Logs (was Agents & Autonomy).
    id: 'tool-policy-diagnostics',
    titleKey: 'devOptions.diagnostics',
    descriptionKey: 'devOptions.toolPolicyDiagnosticsDesc',
    section: 'developer',
    devOnly: true,
    navGroup: 'diagnosticsLogs',
  },
  // Automation & Integrations (debug)
  // mcp-server moved to Connections → MCP → Clients; the slug redirects.
  // dev-workflow (the cron-based GitHub dev-automation panel) was retired —
  // superseded by first-level Workflows (/flows) and the skills workflow runner.
  // Composio trigger-triage config merged into the Connections Composio page.
  // Agent Chat is a chip on the Connections → LLM page; the retired
  // local-model-debug slug redirects there (settingsRouteElements.tsx).
  // skills-runner moved to Connections → Skills → Runner; the slug redirects.
  // The dev-only "Build / version info" alias was removed: it opened the same
  // About page, so dev builds listed two sidebar entries for one page. About's
  // search keywords already cover "build" and "version".

  // Token savings (TokenJuice compression settings + savings) is now the
  // "Token savings" tab of the merged Usage & limits surface on Connections —
  // the standalone token-usage entry was retired (route redirects there).

  // =========================================================================
  // INTENTIONALLY HIDDEN / DEEP-LINK ONLY (not surfaced in any menu)
  // =========================================================================
  {
    // search: web search providers (managed Exa and Gemini, plus bring-your-own-key
    // providers), the per-role provider order, and the allowed-websites list.
    // Surfaced on the Connections page (Intelligence group); route kept for
    // deep-link compatibility but no longer in the settings sidebar.
    id: 'search',
    titleKey: 'settings.search.title',
    section: 'developer',
    devOnly: true,
    searchKeywords: [
      'search',
      'web',
      'provider',
      'exa',
      'gemini',
      'brave',
      'tavily',
      'searxng',
      'answer',
      'research',
    ],
  },
  {
    // permissions: moved to developer options, not a standalone home entry.
    id: 'permissions',
    titleKey: 'settings.assistant.permissions',
    section: 'developer',
    hiddenDeepLink: true,
    devOnly: true,
  },
  {
    // approval-history: leaf under agent-access, deep-link only.
    id: 'approval-history',
    titleKey: 'settings.approvalHistory.title',
    section: 'agents',
    searchKeywords: ['approval', 'history', 'permission', 'audit'],
    navGroup: 'security',
    navOrder: 3,
  },
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/** Returns the route slug for an entry (falls back to `id`). */
export const entryRoute = (entry: SettingsRegistryEntry): string => entry.route ?? entry.id;

/** All entries that belong to a given section (excluding hidden deep-links). */
export const entriesForSection = (section: SettingsSection): SettingsRegistryEntry[] =>
  SETTINGS_ROUTE_REGISTRY.filter(e => e.section === section && !e.hiddenDeepLink);

/** Lookup by id — returns undefined if not found. */
export const findEntryById = (id: string): SettingsRegistryEntry | undefined =>
  SETTINGS_ROUTE_REGISTRY.find(e => e.id === id);

/** Lookup by route slug — returns the first match (ids usually equal routes). */
export const findEntryByRoute = (route: string): SettingsRegistryEntry | undefined =>
  SETTINGS_ROUTE_REGISTRY.find(e => entryRoute(e) === route);

// ---------------------------------------------------------------------------
// Sidebar helpers (two-pane layout)
// ---------------------------------------------------------------------------

interface SettingsSidebarGroup {
  group: SettingsNavGroup;
  entries: SettingsRegistryEntry[];
}

/** Ordered sidebar groups with their (ordered, visible) entries. */
export const sidebarGroups = (): SettingsSidebarGroup[] =>
  NAV_GROUP_ORDER.map(group => ({
    group,
    entries: SETTINGS_ROUTE_REGISTRY.filter(e => e.navGroup === group && !e.hiddenDeepLink).sort(
      (a, b) => (a.navOrder ?? 0) - (b.navOrder ?? 0)
    ),
  })).filter(g => g.entries.length > 0);

/**
 * Resolves the sidebar entry id to highlight for a given route id. Follows
 * `navParent` chains; routes under the developer section highlight the
 * Developer & Diagnostics entry.
 */
export const resolveSidebarId = (routeId: string): string | undefined => {
  const entry = findEntryById(routeId) ?? findEntryByRoute(routeId);
  if (!entry) return undefined;
  if (entry.navGroup) return entry.id;
  if (entry.navParent) {
    return resolveSidebarId(entry.navParent) ?? entry.navParent;
  }
  if (entry.section === 'developer') return 'developer-options';
  return undefined;
};

/**
 * Sub-nav family for a sidebar entry: the entry itself followed by its
 * visible children. Returns [] when the entry has no children (no sub-nav
 * row is rendered).
 */
export const subNavSiblings = (sidebarId: string): SettingsRegistryEntry[] => {
  const parent = findEntryById(sidebarId);
  if (!parent?.navGroup) return [];
  const children = SETTINGS_ROUTE_REGISTRY.filter(
    e => e.navParent === sidebarId && !e.hiddenDeepLink && !e.devOnly
  );
  return children.length > 0 ? [parent, ...children] : [];
};

// Debug log: confirm registry loaded.
if (typeof window !== 'undefined') {
  log('route registry loaded — %d entries', SETTINGS_ROUTE_REGISTRY.length);
}
