import { useCallback } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

export type SettingsRoute =
  | 'home'
  | 'connections'
  | 'messaging'
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
  | 'tauri-commands'
  | 'memory-debug';

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
    if (path.includes('/settings/connections')) return 'connections';
    if (path.includes('/settings/messaging')) return 'messaging';
    if (path.includes('/settings/privacy')) return 'privacy';
    if (path.includes('/settings/profile')) return 'profile';
    if (path.includes('/settings/advanced')) return 'advanced';
    if (path.includes('/settings/billing')) return 'billing';
    if (path.includes('/settings/developer-options')) return 'developer-options';
    if (path.includes('/settings/accessibility')) return 'accessibility';
    if (path.includes('/settings/skills')) return 'skills';
    if (path.includes('/settings/ai')) return 'ai';
    if (path.includes('/settings/local-model')) return 'local-model';
    if (path.includes('/settings/tauri-commands')) return 'tauri-commands';
    if (path.includes('/settings/memory-debug')) return 'memory-debug';
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

  const developerSubRoutes: SettingsRoute[] = [
    'skills',
    'ai',
    'local-model',
    'tauri-commands',
    'memory-debug',
  ];

  const navigateBack = useCallback(() => {
    if (currentRoute === 'home') {
      navigate('/home');
    } else if (currentRoute === 'team-members' || currentRoute === 'team-invites') {
      // Check if we're in team management context (has teamId in URL)
      const teamManageMatch = location.pathname.match(/\/team\/manage\/([^/]+)/);
      if (teamManageMatch) {
        const teamId = teamManageMatch[1];
        navigate(`/settings/team/manage/${teamId}`);
      } else {
        navigate('/settings/team');
      }
    } else if (location.pathname.includes('/team/manage/')) {
      navigate('/settings/team');
    } else if (developerSubRoutes.includes(currentRoute as SettingsRoute)) {
      navigate('/settings/developer-options');
    } else {
      navigate('/settings');
    }
  }, [navigate, currentRoute, location.pathname]);

  const closeSettings = useCallback(() => {
    navigate('/home');
  }, [navigate]);

  return {
    currentRoute,
    navigateToSettings,
    navigateToTeamManagement,
    navigateBack,
    closeSettings,
  };
};
