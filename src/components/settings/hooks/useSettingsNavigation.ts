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
  | 'team-invites';

interface SettingsNavigationHook {
  currentRoute: SettingsRoute;
  navigateToSettings: (route?: SettingsRoute) => void;
  navigateBack: () => void;
  closeSettings: () => void;
}

export const useSettingsNavigation = (): SettingsNavigationHook => {
  const navigate = useNavigate();
  const location = useLocation();

  // Determine current settings route from URL
  const getCurrentRoute = (): SettingsRoute => {
    const path = location.pathname;
    if (path.includes('/settings/team/members')) return 'team-members';
    if (path.includes('/settings/team/invites')) return 'team-invites';
    if (path.includes('/settings/team')) return 'team';
    if (path.includes('/settings/connections')) return 'connections';
    if (path.includes('/settings/messaging')) return 'messaging';
    if (path.includes('/settings/privacy')) return 'privacy';
    if (path.includes('/settings/profile')) return 'profile';
    if (path.includes('/settings/advanced')) return 'advanced';
    if (path.includes('/settings/billing')) return 'billing';
    return 'home';
  };

  const currentRoute = getCurrentRoute();

  const navigateToSettings = useCallback(
    (route: SettingsRoute = 'home') => {
      if (route === 'home') {
        navigate('/settings');
      } else {
        navigate(`/settings/${route}`);
      }
    },
    [navigate]
  );

  const navigateBack = useCallback(() => {
    if (currentRoute === 'home') {
      navigate('/home');
    } else if (currentRoute === 'team-members' || currentRoute === 'team-invites') {
      navigate('/settings/team');
    } else {
      navigate('/settings');
    }
  }, [navigate, currentRoute]);

  const closeSettings = useCallback(() => {
    navigate('/home');
  }, [navigate]);

  return { currentRoute, navigateToSettings, navigateBack, closeSettings };
};
