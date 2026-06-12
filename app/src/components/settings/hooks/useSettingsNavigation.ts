// [settings] navigation hook — route resolution and breadcrumb derivation.
// Uses the settingsRouteRegistry as the single source of truth so that every
// registered route automatically yields a correct breadcrumb trail without
// maintaining a parallel switch-statement.
import debug from 'debug';
import { useCallback } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

import {
  entryRoute,
  findEntryByRoute,
  SETTINGS_ROUTE_REGISTRY,
  type SettingsSection,
} from '../settingsRouteRegistry';

const log = debug('settings:nav');

// ---------------------------------------------------------------------------
// SettingsRoute type — derived from the registry so it stays in sync.
// ---------------------------------------------------------------------------

export type SettingsRoute =
  | 'home'
  | 'agents'
  | 'agents-settings'
  | 'agent-access'
  | 'account'
  | 'features'
  | 'cron-jobs'
  | 'screen-intelligence'
  | 'autocomplete'
  | 'privacy'
  | 'billing'
  | 'team'
  | 'team-members'
  | 'team-invites'
  | 'developer-options'
  | 'autonomy'
  | 'ai'
  | 'llm'
  | 'voice'
  | 'tools'
  | 'memory-data'
  | 'memory-sync'
  | 'memory-debug'
  | 'crypto'
  | 'recovery-phrase'
  | 'wallet-balances'
  | 'webhooks-debug'
  | 'agent-chat'
  | 'screen-awareness-debug'
  | 'autocomplete-debug'
  | 'voice-debug'
  | 'local-model-debug'
  | 'notifications'
  | 'notifications-hub'
  | 'notification-routing'
  | 'mascot'
  | 'persona'
  | 'appearance'
  | 'approval-history'
  | 'intelligence'
  | 'webhooks-triggers'
  | 'composio-triggers'
  | 'composio-routing'
  | 'composio'
  | 'task-sources'
  | 'tasks'
  | 'mcp-server'
  | 'dev-workflow'
  | 'sandbox-settings'
  | 'permissions'
  | 'activity-level'
  | 'devices'
  | 'heartbeat'
  | 'security'
  | 'migration'
  | 'companion'
  | 'embeddings'
  | 'ledger-usage'
  | 'cost-dashboard'
  | 'search'
  | 'skills-runner'
  | 'event-log'
  | 'model-health'
  | 'analysis-views'
  | 'tool-policy-diagnostics'
  | 'about';

export interface BreadcrumbItem {
  label: string;
  onClick?: () => void;
}

interface SettingsNavigationHook {
  currentRoute: SettingsRoute;
  navigateToSettings: (route?: SettingsRoute | string) => void;
  navigateToTeamManagement: (teamId: string) => void;
  navigateBack: () => void;
  closeSettings: () => void;
  breadcrumbs: BreadcrumbItem[];
}

// ---------------------------------------------------------------------------
// Route extraction
//
// Prior implementation used `path.includes()` which is fragile against
// substring collisions (e.g. '/settings/ai' matching '/settings/ai-debug').
// We now extract the slug via an exact-segment split so each path maps to
// exactly one route, then fall back to the registry for known routes.
// ---------------------------------------------------------------------------

/** Extract the settings sub-path from a full pathname. */
const extractSettingsSlug = (pathname: string): string => {
  // Strip the leading /settings/ and take the first path segment.
  // e.g. /settings/agents/edit/123 → 'agents'
  // e.g. /settings/team/manage/456/members → 'team/manage/456/members'
  const match = /^\/settings\/(.+)$/.exec(pathname);
  if (!match) return '';
  return match[1];
};

const getCurrentRoute = (pathname: string): SettingsRoute => {
  const slug = extractSettingsSlug(pathname);
  if (!slug) return 'home';

  // --- special-cased team sub-routes (dynamic segments) ---
  if (/^team\/manage\/.+\/members/.test(slug)) return 'team-members';
  if (/^team\/manage\/.+\/invites/.test(slug)) return 'team-invites';
  if (/^team\/manage\//.test(slug)) return 'team';
  if (/^team\/members/.test(slug)) return 'team-members';
  if (/^team\/invites/.test(slug)) return 'team-invites';
  if (/^team(\/|$)/.test(slug)) return 'team';
  // --- agent editor sub-routes ---
  if (/^agents\/(new|edit)/.test(slug)) return 'agents';

  // --- exact first-segment lookup via registry ---
  const firstSegment = slug.split('/')[0];

  // Try to find the route by first segment first (most routes are single-segment).
  const entry = findEntryByRoute(firstSegment);
  if (entry) {
    log('getCurrentRoute: %s → %s', pathname, entry.id);
    return entry.id as SettingsRoute;
  }

  // A few routes have ids that don't match their URL segment (build-info → about).
  // Check all registry entries whose resolved route matches.
  const byRoute = SETTINGS_ROUTE_REGISTRY.find(e => entryRoute(e) === firstSegment);
  if (byRoute) {
    log('getCurrentRoute (via route alias): %s → %s', pathname, byRoute.id);
    return byRoute.id as SettingsRoute;
  }

  // Legacy redirect targets that don't have a registry entry.
  if (firstSegment === 'notification-routing') return 'notification-routing';

  log('getCurrentRoute: unknown slug "%s", defaulting to home', firstSegment);
  return 'home';
};

// ---------------------------------------------------------------------------
// Section → breadcrumb label mapping (static, no i18n hook dependency).
// Breadcrumb labels are intentionally English-only for now (the existing
// implementation was also English). A future pass can thread the translator.
// ---------------------------------------------------------------------------

const SECTION_LABEL: Record<SettingsSection, string> = {
  home: 'Settings',
  account: 'Account',
  ai: 'AI & Models',
  agents: 'Agents',
  features: 'Features',
  composio: 'Integrations',
  crypto: 'Crypto',
  notifications: 'Notifications',
  developer: 'Developer Options',
};

export const useSettingsNavigation = (): SettingsNavigationHook => {
  const navigate = useNavigate();
  const location = useLocation();

  const goBackWithFallback = useCallback(
    (fallbackPath: string) => {
      const historyState = window.history.state as { idx?: number } | null;
      if (typeof historyState?.idx === 'number' && historyState.idx > 0) {
        navigate(-1);
        return;
      }
      navigate(fallbackPath);
    },
    [navigate]
  );

  const currentRoute = getCurrentRoute(location.pathname);

  const navigateToSettings = useCallback(
    (route: SettingsRoute | string = 'home') => {
      if (route === 'home') {
        navigate('/settings');
      } else {
        navigate(`/settings/${route}`);
      }
    },
    [navigate]
  );

  const navigateToTeamManagement = useCallback(
    (teamId: string) => {
      navigate(`/settings/team/manage/${teamId}`);
    },
    [navigate]
  );

  const navigateBack = useCallback(() => {
    if (currentRoute === 'home') {
      goBackWithFallback('/home');
      return;
    }
    goBackWithFallback('/settings');
  }, [currentRoute, goBackWithFallback]);

  const closeSettings = useCallback(() => {
    goBackWithFallback('/home');
  }, [goBackWithFallback]);

  // -------------------------------------------------------------------------
  // Breadcrumbs — derived from the registry.
  //
  // The root crumb is always "Settings" (pointing to /settings).
  // Section pages (section === 'home') trail: [Settings].
  // Leaf panels trail:  [Settings] > [Section label].
  // Special multi-level trails (team sub-pages, approval-history) are handled
  // explicitly below.
  // -------------------------------------------------------------------------

  const settingsCrumb: BreadcrumbItem = { label: 'Settings', onClick: () => navigate('/settings') };

  const getBreadcrumbs = (): BreadcrumbItem[] => {
    if (currentRoute === 'home') return [];

    // Special cases with deeper trails.
    if (currentRoute === 'team-members' || currentRoute === 'team-invites') {
      return [
        settingsCrumb,
        { label: SECTION_LABEL.account, onClick: () => navigate('/settings/account') },
        { label: 'Team', onClick: () => navigate('/settings/team') },
      ];
    }

    if (currentRoute === 'approval-history') {
      return [
        settingsCrumb,
        { label: SECTION_LABEL.agents, onClick: () => navigate('/settings/agents-settings') },
        { label: 'Agent access', onClick: () => navigate('/settings/agent-access') },
      ];
    }

    // Notification preferences panel nests under notifications-hub.
    if (currentRoute === 'notifications') {
      return [
        settingsCrumb,
        {
          label: SECTION_LABEL.notifications,
          onClick: () => navigate('/settings/notifications-hub'),
        },
      ];
    }

    // Legacy redirect target — kept working but mapped to developer.
    if (currentRoute === 'notification-routing') {
      return [
        settingsCrumb,
        { label: SECTION_LABEL.developer, onClick: () => navigate('/settings/developer-options') },
      ];
    }

    // Look up the entry in the registry using the current route.
    // The currentRoute is the entry id; try by id first, then by resolved route.
    const entry =
      SETTINGS_ROUTE_REGISTRY.find(e => e.id === currentRoute) ??
      SETTINGS_ROUTE_REGISTRY.find(e => entryRoute(e) === currentRoute);

    if (!entry) {
      log('breadcrumbs: no registry entry for "%s"', currentRoute);
      return [settingsCrumb];
    }

    // Home-level entries (section === 'home') are top-level section pages.
    if (entry.section === 'home') {
      return [settingsCrumb];
    }

    // Leaf panels: Settings → <section label>.
    const sectionLabel = SECTION_LABEL[entry.section];
    const sectionRoute = sectionRouteForSection(entry.section);

    return [settingsCrumb, { label: sectionLabel, onClick: () => navigate(sectionRoute) }];
  };

  const breadcrumbs = getBreadcrumbs();

  return {
    currentRoute,
    navigateToSettings,
    navigateToTeamManagement,
    navigateBack,
    closeSettings,
    breadcrumbs,
  };
};

// ---------------------------------------------------------------------------
// Helper: canonical section-page route for a given section.
// ---------------------------------------------------------------------------

const sectionRouteForSection = (section: SettingsSection): string => {
  switch (section) {
    case 'account':
      return '/settings/account';
    case 'ai':
      return '/settings/ai';
    case 'agents':
      return '/settings/agents-settings';
    case 'features':
      return '/settings/features';
    case 'composio':
      return '/settings/composio';
    case 'crypto':
      return '/settings/crypto';
    case 'notifications':
      return '/settings/notifications-hub';
    case 'developer':
      return '/settings/developer-options';
    case 'home':
      return '/settings';
  }
};
