import { useCallback } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

export type SettingsRoute =
  | 'home'
  | 'account'
  | 'automation'
  | 'ai-tools'
  | 'connections'
  | 'messaging'
  | 'cron-jobs'
  | 'screen-intelligence'
  | 'autocomplete'
  | 'privacy'
  | 'billing'
  | 'team'
  | 'team-members'
  | 'team-invites'
  | 'developer-options'
  | 'accessibility'
  | 'ai'
  | 'local-model'
  | 'voice'
  | 'memory-data'
  | 'memory-debug'
  | 'recovery-phrase'
  | 'webhooks-debug'
  | 'agent-chat';

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

  // Determine current settings route from URL
  const getCurrentRoute = (): SettingsRoute => {
    const path = location.pathname;
    // Check specific team management paths first (more specific)
    if (path.includes('/settings/team/manage/') && path.includes('/members')) return 'team-members';
    if (path.includes('/settings/team/manage/') && path.includes('/invites')) return 'team-invites';
    if (path.includes('/settings/team/manage/')) return 'team';
    // Then check regular team paths (less specific)
    if (path.includes('/settings/team/members')) return 'team-members';
    if (path.includes('/settings/team/invites')) return 'team-invites';
    if (path.includes('/settings/team')) return 'team';
    if (path.includes('/settings/account')) return 'account';
    if (path.includes('/settings/automation')) return 'automation';
    if (path.includes('/settings/ai-tools')) return 'ai-tools';
    if (path.includes('/settings/connections')) return 'connections';
    if (path.includes('/settings/messaging')) return 'messaging';
    if (path.includes('/settings/cron-jobs')) return 'cron-jobs';
    if (path.includes('/settings/screen-intelligence')) return 'screen-intelligence';
    if (path.includes('/settings/autocomplete')) return 'autocomplete';
    if (path.includes('/settings/privacy')) return 'privacy';
    if (path.includes('/settings/billing')) return 'billing';
    if (path.includes('/settings/developer-options')) return 'developer-options';
    if (path.includes('/settings/accessibility')) return 'accessibility';
    if (path.includes('/settings/ai')) return 'ai';
    if (path.includes('/settings/local-model')) return 'local-model';
    if (path.includes('/settings/voice')) return 'voice';
    if (path.includes('/settings/memory-data')) return 'memory-data';
    if (path.includes('/settings/memory-debug')) return 'memory-debug';
    if (path.includes('/settings/webhooks-debug')) return 'webhooks-debug';
    if (path.includes('/settings/recovery-phrase')) return 'recovery-phrase';
    if (path.includes('/settings/agent-chat')) return 'agent-chat';
    return 'home';
  };

  const currentRoute = getCurrentRoute();

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

  const settingsCrumb: BreadcrumbItem = { label: 'Settings', onClick: () => navigate('/settings') };

  const accountCrumb: BreadcrumbItem = {
    label: 'Account & Security',
    onClick: () => navigate('/settings/account'),
  };

  const automationCrumb: BreadcrumbItem = {
    label: 'Automation & Channels',
    onClick: () => navigate('/settings/automation'),
  };

  const aiToolsCrumb: BreadcrumbItem = {
    label: 'AI & Skills',
    onClick: () => navigate('/settings/ai-tools'),
  };

  const teamCrumb: BreadcrumbItem = { label: 'Team', onClick: () => navigate('/settings/team') };

  const developerCrumb: BreadcrumbItem = {
    label: 'Developer Options',
    onClick: () => navigate('/settings/developer-options'),
  };

  const getBreadcrumbs = (): BreadcrumbItem[] => {
    switch (currentRoute) {
      // Section pages
      case 'account':
      case 'automation':
      case 'ai-tools':
        return [settingsCrumb];

      // Top-level billing leaf (promoted out of Account & Security)
      case 'billing':
        return [settingsCrumb];

      // Leaf panels under account
      case 'recovery-phrase':
      case 'team':
      case 'connections':
        return [settingsCrumb, accountCrumb];

      // Leaf panels under automation
      case 'accessibility':
      case 'screen-intelligence':
      case 'autocomplete':
      case 'messaging':
      case 'cron-jobs':
        return [settingsCrumb, automationCrumb];

      // Leaf panels under ai-tools
      case 'voice':
      case 'local-model':
      case 'ai':
        return [settingsCrumb, aiToolsCrumb];

      // Team sub-pages
      case 'team-members':
      case 'team-invites':
        return [settingsCrumb, accountCrumb, teamCrumb];

      // Developer sub-pages
      case 'webhooks-debug':
      case 'memory-data':
      case 'memory-debug':
        return [settingsCrumb, developerCrumb];

      // Other leaf pages
      case 'privacy':
      case 'agent-chat':
      case 'developer-options':
        return [settingsCrumb];

      case 'home':
      default:
        return [];
    }
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
