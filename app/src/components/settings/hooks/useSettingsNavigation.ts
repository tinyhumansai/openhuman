// [settings] navigation hook — route resolution for the two-pane settings
// layout. Uses the settingsRouteRegistry as the single source of truth so
// every registered route resolves without a parallel switch-statement.
import debug from 'debug';
import { useCallback } from 'react';
import { type To, useLocation, useNavigate } from 'react-router-dom';

import { settingsNavState } from '../modal/settingsOverlay';
import { entryRoute, findEntryByRoute, SETTINGS_ROUTE_REGISTRY } from '../settingsRouteRegistry';

const log = debug('settings:nav');

// ---------------------------------------------------------------------------
// SettingsRoute type — derived from the registry so it stays in sync.
// ---------------------------------------------------------------------------

export type SettingsRoute =
  | 'home'
  | 'agents'
  | 'agent-access'
  | 'account'
  | 'cron-jobs'
  | 'screen-intelligence'
  | 'desktop-agent'
  | 'autocomplete'
  | 'privacy'
  | 'billing'
  | 'team'
  | 'team-members'
  | 'team-invites'
  | 'developer-options'
  | 'llm'
  | 'voice'
  | 'tools'
  | 'memory-data'
  | 'memory-sync'
  | 'memory-debug'
  | 'recovery-phrase'
  | 'wallet-balances'
  | 'webhooks-debug'
  | 'agent-chat'
  | 'screen-awareness-debug'
  | 'autocomplete-debug'
  | 'voice-debug'
  | 'local-model-debug'
  | 'notifications'
  | 'notification-routing'
  | 'personality'
  | 'appearance'
  | 'approval-history'
  | 'intelligence'
  | 'integrations'
  | 'composio-triggers'
  | 'tasks'
  | 'mcp-server'
  | 'dev-workflow'
  | 'sandbox-settings'
  | 'permissions'
  | 'activity-level'
  | 'devices'
  | 'usage'
  | 'security'
  | 'migration'
  | 'companion'
  | 'meetings'
  | 'embeddings'
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
      // Preserve the modal's backdrop (desktop) across panel-to-panel nav.
      const target = route === 'home' ? '/settings' : `/settings/${route}`;
      navigate(target, settingsNavState(location));
    },
    [navigate, location]
  );

  const navigateToTeamManagement = useCallback(
    (teamId: string) => {
      navigate(`/settings/team/manage/${teamId}`, settingsNavState(location));
    },
    [navigate, location]
  );

  const navigateBack = useCallback(() => {
    if (currentRoute === 'home') {
      goBackWithFallback('/home');
      return;
    }
    goBackWithFallback('/settings');
  }, [currentRoute, goBackWithFallback]);

  const closeSettings = useCallback(() => {
    // On desktop the modal was opened over a page (backgroundLocation); return
    // there. Otherwise fall back to /home.
    const background = (location.state as { backgroundLocation?: To } | null)?.backgroundLocation;
    // replace so pressing Back after closing doesn't reopen the Settings modal.
    navigate(background ?? '/home', { replace: true });
  }, [navigate, location.state]);

  // -------------------------------------------------------------------------
  // Breadcrumbs — derived from the registry.
  //
  // Breadcrumbs were replaced by the two-pane sidebar — the trail is no longer
  // rendered anywhere. The field is kept (always empty) so the ~50 panel call
  // sites keep compiling until the prop is mechanically removed.
  // -------------------------------------------------------------------------

  const breadcrumbs: BreadcrumbItem[] = [];

  return {
    currentRoute,
    navigateToSettings,
    navigateToTeamManagement,
    navigateBack,
    closeSettings,
    breadcrumbs,
  };
};
