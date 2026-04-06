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
  | 'profile'
  | 'advanced'
  | 'billing'
  | 'team'
  | 'team-members'
  | 'team-invites'
  | 'developer-options'
  | 'accessibility'
  | 'skills'
  | 'ai'
  | 'local-model'
  | 'memory-debug'
  | 'recovery-phrase'
  | 'webhooks-debug'
  | 'agent-chat';

interface SettingsNavigationHook {
  currentRoute: SettingsRoute;
  navigateToSettings: (route?: SettingsRoute | string) => void;
  navigateToTeamManagement: (teamId: string) => void;
  navigateBack: () => void;
  closeSettings: () => void;
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
    if (path.includes('/settings/profile')) return 'profile';
    if (path.includes('/settings/advanced')) return 'advanced';
    if (path.includes('/settings/billing')) return 'billing';
    if (path.includes('/settings/developer-options')) return 'developer-options';
    if (path.includes('/settings/accessibility')) return 'accessibility';
    if (path.includes('/settings/skills')) return 'skills';
    if (path.includes('/settings/ai')) return 'ai';
    if (path.includes('/settings/local-model')) return 'local-model';
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

  return {
    currentRoute,
    navigateToSettings,
    navigateToTeamManagement,
    navigateBack,
    closeSettings,
  };
};
