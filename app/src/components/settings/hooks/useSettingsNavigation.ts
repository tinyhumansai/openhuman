import { useCallback } from 'react';
import { useLocation, useNavigate } from 'react-router-dom';

export type SettingsRoute =
  | 'home'
  | 'agents'
  | 'agents-settings'
  | 'agent-access'
  | 'account'
  | 'features'
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
  | 'autonomy'
  | 'ai'
  | 'llm'
  | 'voice'
  | 'tools'
  | 'memory-data'
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
  | 'mcp-server'
  | 'dev-workflow'
  | 'devices';

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
    if (path.includes('/settings/features')) return 'features';
    if (path.includes('/settings/messaging')) return 'messaging';
    if (path.includes('/settings/cron-jobs')) return 'cron-jobs';
    if (path.includes('/settings/screen-awareness-debug')) return 'screen-awareness-debug';
    if (path.includes('/settings/screen-intelligence')) return 'screen-intelligence';
    if (path.includes('/settings/autocomplete-debug')) return 'autocomplete-debug';
    if (path.includes('/settings/autocomplete')) return 'autocomplete';
    if (path.includes('/settings/privacy')) return 'privacy';
    if (path.includes('/settings/billing')) return 'billing';
    if (path.includes('/settings/developer-options')) return 'developer-options';
    if (path.includes('/settings/autonomy')) return 'autonomy';
    if (path.includes('/settings/llm')) return 'llm';
    if (path.includes('/settings/ai')) return 'ai';
    if (path.includes('/settings/local-model-debug')) return 'local-model-debug';
    if (path.includes('/settings/voice-debug')) return 'voice-debug';
    if (path.includes('/settings/voice')) return 'voice';
    if (path.includes('/settings/tools')) return 'tools';
    if (path.includes('/settings/memory-data')) return 'memory-data';
    if (path.includes('/settings/memory-debug')) return 'memory-debug';
    if (path.includes('/settings/webhooks-debug')) return 'webhooks-debug';
    if (path.includes('/settings/webhooks-triggers')) return 'webhooks-triggers';
    if (path.includes('/settings/composio-triggers')) return 'composio-triggers';
    if (path.includes('/settings/composio-routing')) return 'composio-routing';
    if (path.includes('/settings/intelligence')) return 'intelligence';
    if (path.includes('/settings/crypto')) return 'crypto';
    if (path.includes('/settings/recovery-phrase')) return 'recovery-phrase';
    if (path.includes('/settings/wallet-balances')) return 'wallet-balances';
    if (path.includes('/settings/agent-chat')) return 'agent-chat';
    // Notification routes must be checked in specificity order so the more
    // specific `notification-routing` path doesn't get swallowed by the
    // shorter `notifications` prefix.
    if (path.includes('/settings/notification-routing')) return 'notification-routing';
    // `notifications-hub` must be checked before the shorter `notifications`
    // prefix (the tabbed settings panel) so it isn't swallowed.
    if (path.includes('/settings/notifications-hub')) return 'notifications-hub';
    if (path.includes('/settings/notifications')) return 'notifications';
    if (path.includes('/settings/devices')) return 'devices';
    if (path.includes('/settings/mascot')) return 'mascot';
    if (path.includes('/settings/persona')) return 'persona';
    if (path.includes('/settings/appearance')) return 'appearance';
    // `approval-history` is an explicit leaf route under Agent access; it has a
    // distinct prefix from `agent-access`, so ordering between them is cosmetic.
    if (path.includes('/settings/approval-history')) return 'approval-history';
    // `agents-settings` (the Agents section page) must be checked before the
    // shorter `agents` (the manage-agents registry panel) so it isn't swallowed.
    if (path.includes('/settings/agents-settings')) return 'agents-settings';
    if (path.includes('/settings/agent-access')) return 'agent-access';
    if (path.includes('/settings/agents')) return 'agents';
    if (path.includes('/settings/mcp-server')) return 'mcp-server';
    if (path.includes('/settings/dev-workflow')) return 'dev-workflow';
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
    label: 'Account',
    onClick: () => navigate('/settings/account'),
  };

  const featuresCrumb: BreadcrumbItem = {
    label: 'Features',
    onClick: () => navigate('/settings/features'),
  };

  const aiCrumb: BreadcrumbItem = { label: 'AI', onClick: () => navigate('/settings/ai') };

  const teamCrumb: BreadcrumbItem = { label: 'Team', onClick: () => navigate('/settings/team') };

  const developerCrumb: BreadcrumbItem = {
    label: 'Developer Options',
    onClick: () => navigate('/settings/developer-options'),
  };

  const agentAccessCrumb: BreadcrumbItem = {
    label: 'Agent access',
    onClick: () => navigate('/settings/agent-access'),
  };

  const agentsCrumb: BreadcrumbItem = {
    label: 'Agents',
    onClick: () => navigate('/settings/agents-settings'),
  };

  const cryptoCrumb: BreadcrumbItem = {
    label: 'Crypto',
    onClick: () => navigate('/settings/crypto'),
  };

  const notificationsHubCrumb: BreadcrumbItem = {
    label: 'Notifications',
    onClick: () => navigate('/settings/notifications-hub'),
  };

  const getBreadcrumbs = (): BreadcrumbItem[] => {
    switch (currentRoute) {
      // Section pages
      case 'account':
      case 'features':
      case 'ai':
      case 'agents-settings':
      case 'crypto':
        return [settingsCrumb];

      // Leaf panels under the Agents section
      case 'agents':
      case 'agent-access':
      case 'autonomy':
      case 'persona':
        return [settingsCrumb, agentsCrumb];

      // Leaf panels under the Crypto section
      case 'recovery-phrase':
      case 'wallet-balances':
        return [settingsCrumb, cryptoCrumb];

      // Leaf panels under account
      case 'team':
      case 'privacy':
        return [settingsCrumb, accountCrumb];

      case 'billing':
        return [settingsCrumb];

      // Leaf panels under features
      case 'screen-intelligence':
      case 'autocomplete':
      case 'messaging':
      case 'tools':
        return [settingsCrumb, featuresCrumb];

      // Leaf panels under AI
      case 'voice':
      case 'llm':
        return [settingsCrumb, aiCrumb];

      // Team sub-pages
      case 'team-members':
      case 'team-invites':
        return [settingsCrumb, accountCrumb, teamCrumb];

      // Developer sub-pages
      case 'agent-chat':
      case 'cron-jobs':
      case 'screen-awareness-debug':
      case 'autocomplete-debug':
      case 'voice-debug':
      case 'local-model-debug':
      case 'webhooks-debug':
      case 'memory-data':
      case 'memory-debug':
      case 'intelligence':
      case 'webhooks-triggers':
      case 'composio-triggers':
      case 'composio-routing':
      case 'notification-routing':
      case 'mcp-server':
      case 'dev-workflow':
      case 'notifications-hub': // Notifications hub section page lives under Advanced.
        return [settingsCrumb, developerCrumb];

      // Developer options section page
      case 'developer-options':
        return [settingsCrumb];

      // Notification preferences panel is a leaf under the Advanced →
      // Notifications hub.
      case 'notifications':
        return [settingsCrumb, developerCrumb, notificationsHubCrumb];

      case 'devices':
        return [settingsCrumb];

      // Mascot appearance panel sits at the top level of Settings.
      case 'mascot':
        return [settingsCrumb];

      // Appearance (theme) panel sits at the top level of Settings.
      case 'appearance':
        return [settingsCrumb];

      // Approval history is a leaf under Agent access, which itself lives under
      // the Agents section — so the trail is Settings → Agents → Agent access.
      case 'approval-history':
        return [settingsCrumb, agentsCrumb, agentAccessCrumb];

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
