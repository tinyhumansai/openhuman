import debug from 'debug';

// ---------------------------------------------------------------------------
// Settings Route Registry
//
// Single declarative source of truth for every navigable settings destination.
// Consumers (SettingsHome, Settings.tsx section arrays, DeveloperOptionsPanel,
// settingsSearchRegistry) derive their menus from here so that a route added
// once automatically appears in navigation, breadcrumbs, and search.
//
// Section values determine the canonical breadcrumb parent:
//   'home'      → top-level home menu entry (Settings breadcrumb only)
//   'account'   → Settings → Account
//   'ai'        → Settings → AI & Models
//   'agents'    → Settings → Agents
//   'features'  → Settings → Features
//   'crypto'    → Settings → Crypto
//   'notifications' → Settings → Notifications
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
  | 'notifications'
  | 'developer';

/**
 * Sidebar groups for the two-pane settings layout, in display order. The former
 * "System" group's Developer & Diagnostics sub-sections are now first-class
 * top-level groups.
 */
export type SettingsNavGroup =
  | 'general'
  | 'assistant'
  | 'data'
  | 'connections'
  | 'knowledgeMemory'
  | 'agentsAutonomy'
  | 'modelsInference'
  | 'automationIntegrations'
  | 'diagnosticsLogs';

export const NAV_GROUP_ORDER: SettingsNavGroup[] = [
  'general',
  'assistant',
  'data',
  'connections',
  'knowledgeMemory',
  'agentsAutonomy',
  'modelsInference',
  'automationIntegrations',
  'diagnosticsLogs',
];

/** i18n keys for the sidebar group labels. */
export const NAV_GROUP_LABEL_KEY: Record<SettingsNavGroup, string> = {
  general: 'settings.navGroups.general',
  assistant: 'settings.navGroups.assistant',
  data: 'settings.navGroups.data',
  connections: 'settings.navGroups.connections',
  // Promoted from the old Developer & Diagnostics sub-sections.
  knowledgeMemory: 'settings.devGroups.knowledgeMemory',
  agentsAutonomy: 'settings.devGroups.agentsAutonomy',
  modelsInference: 'settings.devGroups.modelsInference',
  automationIntegrations: 'settings.devGroups.automationIntegrations',
  diagnosticsLogs: 'settings.devGroups.diagnosticsLogs',
};

export interface SettingsRegistryEntry {
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
    searchKeywords: ['profile', 'sign out', 'logout'],
    navGroup: 'general',
    navOrder: 0,
  },
  {
    // appearance also hosts the display-language selector (formerly an inline
    // row on the old settings home list).
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
      'language',
      'locale',
      'translation',
    ],
    navGroup: 'general',
    navOrder: 1,
  },
  {
    // Theme Studio — full colour + font customization on top of the simple
    // light/dark toggle in Appearance.
    id: 'theme',
    titleKey: 'settings.theme.title',
    descriptionKey: 'settings.theme.menuDesc',
    section: 'home',
    searchKeywords: [
      'theme',
      'color',
      'colour',
      'font',
      'palette',
      'customize',
      'customise',
      'appearance',
      'surface',
      'background',
      'accent',
    ],
    navGroup: 'general',
    navOrder: 2,
  },
  {
    // devices: real pairing panel (the old "Coming Soon" stub was removed).
    id: 'devices',
    titleKey: 'settings.account.devices',
    descriptionKey: 'settings.account.devicesDesc',
    section: 'home',
    searchKeywords: ['mobile', 'phone', 'ios', 'android', 'pair'],
    navGroup: 'general',
    navOrder: 3,
  },

  // --- Assistant group ---
  // The old 'ai' and 'agents-settings' hub pages are retired — their slugs
  // redirect to /settings/llm and /settings/agents.
  {
    // personality: merged Personality & Face page (formerly persona and
    // mascot — those slugs redirect here).
    id: 'personality',
    titleKey: 'settings.personalityFace.title',
    descriptionKey: 'settings.personalityFace.menuDesc',
    section: 'home',
    searchKeywords: [
      'personality',
      'tone',
      'character',
      'persona',
      'face',
      'avatar',
      'mascot',
      'tiny',
    ],
    navGroup: 'assistant',
    navOrder: 2,
  },

  // --- Connections group ---
  // The Integrations settings section was retired — the composio/OAuth grid
  // lives on the Connections page and the task-source/webhook triage surface is
  // no longer used. Screen Awareness, Desktop Agent and Desktop Companion moved
  // to the Connections page's Desktop group; their slugs redirect there.

  // Notifications-hub and crypto hub pages are retired — their slugs redirect
  // to /settings/notifications and /settings/wallet-balances.

  // --- About ---
  {
    id: 'keyboard-shortcuts',
    titleKey: 'shortcuts.title',
    descriptionKey: 'shortcuts.menuDesc',
    section: 'home',
    searchKeywords: ['keyboard', 'shortcuts', 'keys', 'hotkeys', 'bindings', 'cheatsheet'],
    navGroup: 'general',
    navOrder: 98,
  },
  {
    // Core connection — promotes cloud-mode remote-core config (persisted
    // RPC URL + token) into a first-class setting plus a live status
    // indicator (GH-4396). Sits just above About in General.
    id: 'core',
    titleKey: 'settings.core.title',
    descriptionKey: 'settings.core.menuDesc',
    section: 'home',
    searchKeywords: [
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
  {
    id: 'team',
    titleKey: 'pages.settings.account.team',
    descriptionKey: 'pages.settings.account.teamDesc',
    section: 'account',
    searchKeywords: ['members', 'invites', 'organization', 'organisation', 'workspace'],
    navParent: 'account',
  },
  {
    id: 'privacy',
    titleKey: 'pages.settings.account.privacy',
    descriptionKey: 'pages.settings.account.privacyDesc',
    section: 'account',
    searchKeywords: ['telemetry', 'tracking', 'analytics', 'data'],
    navParent: 'account',
  },
  {
    id: 'security',
    titleKey: 'pages.settings.account.security',
    descriptionKey: 'pages.settings.account.securityDesc',
    section: 'account',
    searchKeywords: ['keychain', 'secret', 'password', 'encryption', 'credentials'],
    navParent: 'account',
  },
  {
    id: 'migration',
    titleKey: 'pages.settings.account.migration',
    descriptionKey: 'pages.settings.account.migrationDesc',
    section: 'account',
    searchKeywords: ['import', 'export', 'transfer', 'data'],
    navParent: 'account',
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
    // (formerly heartbeat / ledger-usage). Surfaced on the Connections page
    // (API-keys group); the route redirects there and it's no longer in the
    // settings sidebar. Legacy heartbeat / ledger-usage / cost-dashboard /
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
      'billing',
      'budget',
      'heartbeat',
      'loops',
      'background',
    ],
  },

  // --- Agent profiles (top-level sidebar destination, Assistant group) ---
  {
    // profiles: top-level agent profiles (soul, memory, skills, MCP, connectors).
    // Child routes profiles/new and profiles/edit/:id resolve to this entry.
    id: 'profiles',
    titleKey: 'settings.profiles.title',
    descriptionKey: 'settings.profiles.menuDesc',
    section: 'home',
    searchKeywords: [
      'profile',
      'profiles',
      'agent',
      'soul',
      'memory',
      'skills',
      'mcp',
      'connectors',
    ],
    navGroup: 'assistant',
    navOrder: 3,
  },

  // =========================================================================
  // AGENTS section leaf panels
  // =========================================================================
  {
    id: 'agents',
    titleKey: 'settings.agents.title',
    descriptionKey: 'settings.agents.subtitle',
    section: 'agents',
    searchKeywords: ['agent', 'profiles'],
    navGroup: 'assistant',
    navOrder: 4,
  },
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
    ],
    navParent: 'agents',
  },
  {
    id: 'activity-level',
    titleKey: 'activityLevel.title',
    descriptionKey: 'activityLevel.description',
    section: 'agents',
    searchKeywords: ['background', 'activity', 'subconscious'],
    navParent: 'agents',
  },
  {
    id: 'sandbox-settings',
    titleKey: 'settings.sandbox.title',
    descriptionKey: 'settings.sandbox.menuDesc',
    section: 'agents',
    searchKeywords: ['sandbox', 'jail', 'isolation', 'docker'],
    navParent: 'agents',
  },

  // =========================================================================
  // FEATURES section leaf panels
  // =========================================================================
  {
    // Surfaced on the Connections page (Desktop group); route redirects there.
    // Entry kept for search + deep-link compatibility.
    id: 'screen-intelligence',
    titleKey: 'pages.settings.features.screenAwareness',
    descriptionKey: 'pages.settings.features.screenAwarenessDesc',
    section: 'features',
    searchKeywords: ['screen', 'awareness', 'vision', 'capture'],
  },
  {
    // Surfaced on the Connections page (Desktop group); route redirects there.
    id: 'desktop-agent',
    titleKey: 'settings.desktopAgent.title',
    descriptionKey: 'settings.desktopAgent.description',
    section: 'features',
    searchKeywords: [
      'desktop',
      'agent',
      'automation',
      'permissions',
      'microphone',
      'accessibility',
      'screen recording',
      'input monitoring',
    ],
  },
  {
    id: 'tools',
    titleKey: 'pages.settings.features.tools',
    descriptionKey: 'pages.settings.features.toolsDesc',
    section: 'features',
    searchKeywords: ['tools', 'capabilities', 'functions'],
    navGroup: 'connections',
    navOrder: 3,
  },
  {
    // Surfaced on the Connections page (Desktop group); route redirects there.
    id: 'companion',
    titleKey: 'pages.settings.features.desktopCompanion',
    descriptionKey: 'pages.settings.features.desktopCompanionDesc',
    section: 'features',
    searchKeywords: ['desktop', 'overlay', 'companion'],
  },
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

  // =========================================================================
  // NOTIFICATIONS section leaf panels
  // =========================================================================
  // alerts is an external link (→ /notifications) handled inline in Settings.tsx
  {
    id: 'notifications',
    route: 'notifications',
    titleKey: 'settings.notifications.menuTitle',
    descriptionKey: 'settings.notifications.menuDesc',
    section: 'notifications',
    searchKeywords: ['alerts', 'push', 'preferences', 'routing'],
    navGroup: 'general',
    navOrder: 2,
  },

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
  //   tools, companion, screen-intelligence, voice, embeddings, heartbeat,
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
  // Knowledge & Memory group retired entirely — memory surfaces live on the
  // Brain page (graph / goals / sources / sync / subconscious).
  // voice-debug retired from the settings UI.
  {
    id: 'screen-awareness-debug',
    titleKey: 'settings.developerMenu.screenAwareness.title',
    descriptionKey: 'settings.developerMenu.screenAwareness.desc',
    section: 'developer',
    devOnly: true,
    navGroup: 'modelsInference',
  },
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
  {
    id: 'agentbox',
    titleKey: 'settings.agentbox.title',
    descriptionKey: 'settings.agentbox.desc',
    section: 'developer',
    devOnly: true,
    navGroup: 'modelsInference',
    searchKeywords: ['agentbox', 'gmi', 'maas', 'marketplace'],
  },
  // Automation & Integrations (debug)
  {
    id: 'mcp-server',
    titleKey: 'settings.developerMenu.mcpServer.title',
    descriptionKey: 'settings.developerMenu.mcpServer.desc',
    section: 'developer',
    devOnly: true,
    navGroup: 'automationIntegrations',
    searchKeywords: ['mcp', 'server'],
  },
  // dev-workflow (the cron-based GitHub dev-automation panel) was retired —
  // superseded by first-level Workflows (/flows) and the skills workflow runner.
  {
    id: 'cron-jobs',
    titleKey: 'settings.developerMenu.cronJobs.title',
    descriptionKey: 'settings.developerMenu.cronJobs.desc',
    section: 'developer',
    devOnly: true,
    navGroup: 'automationIntegrations',
    searchKeywords: ['cron', 'schedule', 'jobs'],
  },
  // Composio trigger-triage config merged into the Connections Composio page.
  // Agent Chat + Local Model Debug are now chips on the Connections → LLM page.
  {
    id: 'skills-runner',
    titleKey: 'settings.developerMenu.skillsRunner.title',
    descriptionKey: 'settings.developerMenu.skillsRunner.desc',
    section: 'developer',
    devOnly: true,
    navGroup: 'agentsAutonomy',
  },
  {
    id: 'autocomplete-debug',
    titleKey: 'settings.developerMenu.autocomplete.title',
    descriptionKey: 'settings.developerMenu.autocomplete.desc',
    section: 'developer',
    devOnly: true,
    navGroup: 'modelsInference',
  },
  // Build Info (about page alias in dev menu)
  {
    id: 'build-info',
    route: 'about',
    titleKey: 'settings.buildInfo.title',
    descriptionKey: 'settings.buildInfo.menuDesc',
    section: 'developer',
    devOnly: true,
    navGroup: 'diagnosticsLogs',
  },

  // Token & Cost (TokenJuice compression settings + savings) is now the
  // "Token savings" tab of the merged Usage & limits surface on Connections —
  // the standalone token-usage entry was retired (route redirects there).

  // =========================================================================
  // INTENTIONALLY HIDDEN / DEEP-LINK ONLY (not surfaced in any menu)
  // =========================================================================
  {
    // billing: surfaced in the General group (also opened from the avatar menu).
    id: 'billing',
    titleKey: 'nav.avatarMenu.billing',
    section: 'home',
    searchKeywords: ['billing', 'subscription', 'payment', 'plan', 'invoice'],
    navGroup: 'general',
    navOrder: 4,
    highlight: true,
  },
  {
    // autocomplete: hidden per #717 (route retained for re-enable).
    id: 'autocomplete',
    titleKey: 'settings.developerMenu.autocomplete.title',
    section: 'developer',
    hiddenDeepLink: true,
    devOnly: true,
  },
  {
    // search: web search engine settings (Brave / Google / Tavily provider).
    // Surfaced on the Connections page (Intelligence group); route kept for
    // deep-link compatibility but no longer in the settings sidebar.
    id: 'search',
    titleKey: 'settings.search.title',
    section: 'developer',
    devOnly: true,
    searchKeywords: ['search', 'engine', 'web', 'brave', 'google', 'tavily', 'provider'],
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
    navGroup: 'agentsAutonomy',
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

export interface SettingsSidebarGroup {
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
