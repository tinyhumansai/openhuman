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
//   'composio'  → Settings → Integrations
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
  | 'composio'
  | 'crypto'
  | 'notifications'
  | 'developer';

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
  },
  {
    id: 'appearance',
    titleKey: 'settings.appearance.title',
    descriptionKey: 'settings.appearance.menuDesc',
    section: 'home',
    searchKeywords: ['theme', 'dark', 'light', 'mode', 'color', 'colour'],
  },
  // Language is inline on SettingsHome (no route) — not registered here.
  {
    id: 'devices',
    titleKey: 'settings.account.devices',
    descriptionKey: 'settings.account.devicesDesc',
    section: 'home',
    searchKeywords: ['mobile', 'phone', 'ios', 'android', 'pair'],
  },
  {
    id: 'memory-sync',
    titleKey: 'settings.dataSync.title',
    descriptionKey: 'settings.dataSync.menuDesc',
    section: 'home',
    searchKeywords: ['sync', 'backup', 'data', 'memory'],
  },

  // --- Assistant group (section hubs) ---
  {
    id: 'ai',
    titleKey: 'pages.settings.aiSection.title',
    descriptionKey: 'pages.settings.aiSection.description',
    section: 'home',
    searchKeywords: ['ai', 'models', 'inference', 'llm'],
  },
  {
    id: 'agents-settings',
    titleKey: 'settings.agentsSection.title',
    descriptionKey: 'settings.agentsSection.description',
    section: 'home',
    searchKeywords: ['agents', 'autonomy', 'access'],
  },
  {
    id: 'persona',
    titleKey: 'settings.assistant.personality',
    descriptionKey: 'settings.assistant.personalityDesc',
    section: 'home',
    searchKeywords: ['personality', 'tone', 'character', 'persona'],
  },
  {
    id: 'mascot',
    titleKey: 'settings.assistant.faceMascot',
    descriptionKey: 'settings.assistant.faceMascotDesc',
    section: 'home',
    searchKeywords: ['face', 'avatar', 'tiny', 'character'],
  },

  // --- Features & Integrations group (section hubs) ---
  {
    id: 'features',
    titleKey: 'pages.settings.featuresSection.title',
    descriptionKey: 'pages.settings.featuresSection.description',
    section: 'home',
    searchKeywords: ['features', 'screen', 'tools', 'companion'],
  },
  {
    id: 'composio',
    titleKey: 'pages.settings.composioSection.title',
    descriptionKey: 'pages.settings.composioSection.description',
    section: 'home',
    searchKeywords: ['integrations', 'composio', 'webhooks', 'tasks'],
  },

  // --- Notifications ---
  {
    id: 'notifications-hub',
    titleKey: 'settings.notifications.menuTitle',
    descriptionKey: 'settings.notifications.menuDesc',
    section: 'home',
    searchKeywords: ['alerts', 'push', 'routing'],
  },

  // --- Crypto ---
  {
    id: 'crypto',
    titleKey: 'settings.cryptoSection.title',
    descriptionKey: 'settings.cryptoSection.description',
    section: 'home',
    searchKeywords: ['crypto', 'wallet', 'recovery'],
  },

  // --- About (always visible, no section header) ---
  {
    id: 'about',
    titleKey: 'settings.about',
    descriptionKey: 'settings.aboutDesc',
    section: 'home',
    searchKeywords: ['version', 'build', 'update', 'developer mode'],
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
  },
  {
    id: 'privacy',
    titleKey: 'pages.settings.account.privacy',
    descriptionKey: 'pages.settings.account.privacyDesc',
    section: 'account',
    searchKeywords: ['telemetry', 'tracking', 'analytics', 'data'],
  },
  {
    id: 'security',
    titleKey: 'pages.settings.account.security',
    descriptionKey: 'pages.settings.account.securityDesc',
    section: 'account',
    searchKeywords: ['keychain', 'secret', 'password', 'encryption', 'credentials'],
  },
  {
    id: 'migration',
    titleKey: 'pages.settings.account.migration',
    descriptionKey: 'pages.settings.account.migrationDesc',
    section: 'account',
    searchKeywords: ['import', 'export', 'transfer', 'data'],
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
  },
  {
    id: 'embeddings',
    titleKey: 'pages.settings.ai.embeddings',
    descriptionKey: 'pages.settings.ai.embeddingsDesc',
    section: 'ai',
    searchKeywords: ['vector', 'embedding', 'search'],
  },
  {
    id: 'voice',
    titleKey: 'pages.settings.ai.voice',
    descriptionKey: 'pages.settings.ai.voiceDesc',
    section: 'ai',
    searchKeywords: ['tts', 'stt', 'speech', 'dictation', 'audio'],
  },
  {
    id: 'heartbeat',
    titleKey: 'settings.heartbeat.title',
    descriptionKey: 'settings.heartbeat.desc',
    section: 'ai',
  },
  {
    id: 'ledger-usage',
    titleKey: 'settings.ledgerUsage.title',
    descriptionKey: 'settings.ledgerUsage.desc',
    section: 'ai',
    searchKeywords: ['usage', 'tokens', 'ledger', 'cost'],
  },
  {
    id: 'cost-dashboard',
    titleKey: 'settings.costDashboard.title',
    descriptionKey: 'settings.costDashboard.desc',
    section: 'ai',
    searchKeywords: ['cost', 'spend', 'usage', 'billing'],
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
  },
  {
    id: 'autonomy',
    titleKey: 'settings.developerMenu.autonomy.title',
    descriptionKey: 'settings.developerMenu.autonomy.desc',
    section: 'agents',
    searchKeywords: ['autonomy', 'autonomous'],
  },
  {
    id: 'agent-access',
    titleKey: 'settings.agentAccess.title',
    descriptionKey: 'settings.agentAccess.menuDesc',
    section: 'agents',
    searchKeywords: ['access', 'permissions', 'tier', 'security policy'],
  },
  {
    id: 'activity-level',
    titleKey: 'activityLevel.title',
    descriptionKey: 'activityLevel.description',
    section: 'agents',
    searchKeywords: ['background', 'activity', 'subconscious'],
  },
  {
    id: 'sandbox-settings',
    titleKey: 'settings.sandbox.title',
    descriptionKey: 'settings.sandbox.menuDesc',
    section: 'agents',
    searchKeywords: ['sandbox', 'jail', 'isolation', 'docker'],
  },

  // =========================================================================
  // FEATURES section leaf panels
  // =========================================================================
  {
    id: 'screen-intelligence',
    titleKey: 'pages.settings.features.screenAwareness',
    descriptionKey: 'pages.settings.features.screenAwarenessDesc',
    section: 'features',
    searchKeywords: ['screen', 'awareness', 'vision', 'capture'],
  },
  {
    id: 'tools',
    titleKey: 'pages.settings.features.tools',
    descriptionKey: 'pages.settings.features.toolsDesc',
    section: 'features',
    searchKeywords: ['tools', 'capabilities', 'functions'],
  },
  {
    id: 'companion',
    titleKey: 'pages.settings.features.desktopCompanion',
    descriptionKey: 'pages.settings.features.desktopCompanionDesc',
    section: 'features',
    searchKeywords: ['desktop', 'overlay', 'companion'],
  },

  // =========================================================================
  // COMPOSIO / INTEGRATIONS section leaf panels
  // =========================================================================
  {
    id: 'task-sources',
    titleKey: 'settings.taskSources.title',
    descriptionKey: 'settings.taskSources.subtitle',
    section: 'composio',
    searchKeywords: ['tasks', 'sources', 'inbox'],
  },
  {
    id: 'composio-routing',
    titleKey: 'settings.developerMenu.composioRouting.title',
    descriptionKey: 'settings.developerMenu.composioRouting.desc',
    section: 'composio',
    searchKeywords: ['composio', 'routing', 'integrations'],
  },
  {
    id: 'webhooks-triggers',
    titleKey: 'settings.developerMenu.composeioTriggers.title',
    descriptionKey: 'settings.developerMenu.composeioTriggers.desc',
    section: 'composio',
    searchKeywords: ['webhooks', 'triggers', 'composio'],
  },

  // =========================================================================
  // NOTIFICATIONS section leaf panels
  // =========================================================================
  // alerts is an external link (→ /notifications) handled inline in Settings.tsx
  {
    id: 'notifications',
    route: 'notifications',
    titleKey: 'settings.notificationsHub.settingsItem',
    descriptionKey: 'settings.notificationsHub.settingsItemDesc',
    section: 'notifications',
    searchKeywords: ['alerts', 'push', 'preferences', 'routing'],
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
  },
  {
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
    // developer-options is the section page itself — its breadcrumb is just [Settings].
    id: 'developer-options',
    titleKey: 'settings.developerDiagnostics',
    descriptionKey: 'settings.developerDiagnosticsDesc',
    section: 'home',
    devOnly: true,
    searchKeywords: ['developer', 'diagnostics', 'debug'],
  },
  // Knowledge & Memory
  {
    id: 'intelligence',
    titleKey: 'settings.developerMenu.intelligence.title',
    descriptionKey: 'settings.developerMenu.intelligence.desc',
    section: 'developer',
    devOnly: true,
  },
  {
    id: 'memory-data',
    titleKey: 'devOptions.memoryInspection',
    descriptionKey: 'devOptions.memoryInspectionDesc',
    section: 'developer',
    devOnly: true,
    searchKeywords: ['memory', 'inspect'],
  },
  {
    id: 'memory-debug',
    titleKey: 'devOptions.debugPanels',
    descriptionKey: 'devOptions.debugPanelsDesc',
    section: 'developer',
    devOnly: true,
  },
  {
    id: 'analysis-views',
    titleKey: 'settings.analysisViews.title',
    descriptionKey: 'settings.analysisViews.menuDesc',
    section: 'developer',
    devOnly: true,
  },
  // Diagnostics & Logs
  {
    id: 'voice-debug',
    titleKey: 'settings.developerMenu.voiceDebug.title',
    descriptionKey: 'settings.developerMenu.voiceDebug.desc',
    section: 'developer',
    devOnly: true,
  },
  {
    id: 'screen-awareness-debug',
    titleKey: 'settings.developerMenu.screenAwareness.title',
    descriptionKey: 'settings.developerMenu.screenAwareness.desc',
    section: 'developer',
    devOnly: true,
  },
  {
    id: 'event-log',
    titleKey: 'settings.developerMenu.eventLog.title',
    descriptionKey: 'settings.developerMenu.eventLog.desc',
    section: 'developer',
    devOnly: true,
    searchKeywords: ['events', 'log'],
  },
  {
    id: 'tool-policy-diagnostics',
    titleKey: 'devOptions.diagnostics',
    descriptionKey: 'devOptions.toolPolicyDiagnosticsDesc',
    section: 'developer',
    devOnly: true,
  },
  {
    id: 'model-health',
    titleKey: 'settings.modelHealth.title',
    descriptionKey: 'settings.modelHealth.desc',
    section: 'developer',
    devOnly: true,
  },
  {
    id: 'webhooks-debug',
    titleKey: 'settings.developerMenu.webhooks.title',
    descriptionKey: 'settings.developerMenu.webhooks.desc',
    section: 'developer',
    devOnly: true,
  },
  // Automation & Integrations (debug)
  {
    id: 'mcp-server',
    titleKey: 'settings.developerMenu.mcpServer.title',
    descriptionKey: 'settings.developerMenu.mcpServer.desc',
    section: 'developer',
    devOnly: true,
    searchKeywords: ['mcp', 'server'],
  },
  {
    id: 'dev-workflow',
    titleKey: 'settings.developerMenu.devWorkflow.title',
    descriptionKey: 'settings.developerMenu.devWorkflow.desc',
    section: 'developer',
    devOnly: true,
  },
  {
    id: 'cron-jobs',
    titleKey: 'settings.developerMenu.cronJobs.title',
    descriptionKey: 'settings.developerMenu.cronJobs.desc',
    section: 'developer',
    devOnly: true,
    searchKeywords: ['cron', 'schedule', 'jobs'],
  },
  {
    id: 'tasks',
    titleKey: 'settings.developerMenu.tasks.title',
    descriptionKey: 'settings.developerMenu.tasks.desc',
    section: 'developer',
    devOnly: true,
  },
  {
    // composio-triggers: renders ComposioTriagePanel — debug alias kept under Developer Options.
    id: 'composio-triggers',
    titleKey: 'settings.developerMenu.composio.title',
    descriptionKey: 'settings.developerMenu.composio.desc',
    section: 'developer',
    devOnly: true,
  },
  // Agent debug
  {
    id: 'agent-chat',
    titleKey: 'settings.developerMenu.agentChat.title',
    descriptionKey: 'settings.developerMenu.agentChat.desc',
    section: 'developer',
    devOnly: true,
  },
  {
    id: 'local-model-debug',
    titleKey: 'settings.developerMenu.localModelDebug.title',
    descriptionKey: 'settings.developerMenu.localModelDebug.desc',
    section: 'developer',
    devOnly: true,
  },
  {
    id: 'skills-runner',
    titleKey: 'settings.developerMenu.skillsRunner.title',
    descriptionKey: 'settings.developerMenu.skillsRunner.desc',
    section: 'developer',
    devOnly: true,
  },
  {
    id: 'autocomplete-debug',
    titleKey: 'settings.developerMenu.autocomplete.title',
    descriptionKey: 'settings.developerMenu.autocomplete.desc',
    section: 'developer',
    devOnly: true,
  },
  // Build Info (about page alias in dev menu)
  {
    id: 'build-info',
    route: 'about',
    titleKey: 'settings.buildInfo.title',
    descriptionKey: 'settings.buildInfo.menuDesc',
    section: 'developer',
    devOnly: true,
  },

  // =========================================================================
  // INTENTIONALLY HIDDEN / DEEP-LINK ONLY (not surfaced in any menu)
  // =========================================================================
  {
    // billing: deep-link only — avatar menu navigates here directly.
    // Route retained; no home entry by design.
    id: 'billing',
    titleKey: 'settings.billing.movedToWeb',
    section: 'home',
    hiddenDeepLink: true,
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
    // search: internal search debug panel, not surfaced in the main nav.
    id: 'search',
    titleKey: 'settings.search.title',
    section: 'developer',
    hiddenDeepLink: true,
    devOnly: true,
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
    hiddenDeepLink: true,
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

// Debug log: confirm registry loaded.
if (typeof window !== 'undefined') {
  log('route registry loaded — %d entries', SETTINGS_ROUTE_REGISTRY.length);
}
