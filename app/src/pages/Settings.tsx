import type { ReactNode } from 'react';
import { Navigate, Route, Routes } from 'react-router-dom';

import CostDashboardPanel from '../components/dashboard/CostDashboardPanel';
import LogoutAndClearActions from '../components/settings/LogoutAndClearActions';
import AboutPanel from '../components/settings/panels/AboutPanel';
import AgentAccessPanel from '../components/settings/panels/AgentAccessPanel';
import AgentChatPanel from '../components/settings/panels/AgentChatPanel';
import AgentsPanel from '../components/settings/panels/AgentsPanel';
import AIPanel from '../components/settings/panels/AIPanel';
import AppearancePanel from '../components/settings/panels/AppearancePanel';
import AutocompleteDebugPanel from '../components/settings/panels/AutocompleteDebugPanel';
import AutocompletePanel from '../components/settings/panels/AutocompletePanel';
import AutonomyPanel from '../components/settings/panels/AutonomyPanel';
import BillingPanel from '../components/settings/panels/BillingPanel';
import CompanionPanel from '../components/settings/panels/CompanionPanel';
import ComposioPanel from '../components/settings/panels/ComposioPanel';
import ComposioTriagePanel from '../components/settings/panels/ComposioTriagePanel';
import CronJobsPanel from '../components/settings/panels/CronJobsPanel';
import DeveloperOptionsPanel from '../components/settings/panels/DeveloperOptionsPanel';
import DevicesComingSoonPanel from '../components/settings/panels/DevicesComingSoonPanel';
import DevWorkflowPanel from '../components/settings/panels/DevWorkflowPanel';
import EmbeddingsPanel from '../components/settings/panels/EmbeddingsPanel';
import EventLogPanel from '../components/settings/panels/EventLogPanel';
import HeartbeatPanel from '../components/settings/panels/HeartbeatPanel';
import LedgerUsagePanel from '../components/settings/panels/LedgerUsagePanel';
import LocalModelDebugPanel from '../components/settings/panels/LocalModelDebugPanel';
import MascotPanel from '../components/settings/panels/MascotPanel';
import McpServerPanel from '../components/settings/panels/McpServerPanel';
import MemoryDataPanel from '../components/settings/panels/MemoryDataPanel';
import MemoryDebugPanel from '../components/settings/panels/MemoryDebugPanel';
import MigrationPanel from '../components/settings/panels/MigrationPanel';
import ModelHealthPanel from '../components/settings/panels/ModelHealthPanel';
import NotificationsTabbedPanel from '../components/settings/panels/NotificationsTabbedPanel';
import PersonaPanel from '../components/settings/panels/PersonaPanel';
import PrivacyPanel from '../components/settings/panels/PrivacyPanel';
import RecoveryPhrasePanel from '../components/settings/panels/RecoveryPhrasePanel';
import ScreenAwarenessDebugPanel from '../components/settings/panels/ScreenAwarenessDebugPanel';
import ScreenIntelligencePanel from '../components/settings/panels/ScreenIntelligencePanel';
import SearchPanel from '../components/settings/panels/SearchPanel';
import SkillsRunnerPanel from '../components/settings/panels/SkillsRunnerPanel';
import TaskSourcesPanel from '../components/settings/panels/TaskSourcesPanel';
import TeamInvitesPanel from '../components/settings/panels/TeamInvitesPanel';
import TeamManagementPanel from '../components/settings/panels/TeamManagementPanel';
import TeamMembersPanel from '../components/settings/panels/TeamMembersPanel';
import TeamPanel from '../components/settings/panels/TeamPanel';
import ToolPolicyDiagnosticsPanel from '../components/settings/panels/ToolPolicyDiagnosticsPanel';
import ToolsPanel from '../components/settings/panels/ToolsPanel';
import VoiceDebugPanel from '../components/settings/panels/VoiceDebugPanel';
import VoicePanel from '../components/settings/panels/VoicePanel';
import WebhooksDebugPanel from '../components/settings/panels/WebhooksDebugPanel';
import SettingsHome from '../components/settings/SettingsHome';
import SettingsSectionPage from '../components/settings/SettingsSectionPage';
import { useT } from '../lib/i18n/I18nContext';
import { APP_VERSION } from '../utils/config';
import Intelligence from './Intelligence';
import Webhooks from './Webhooks';

// Icon elements extracted as constants to avoid repeating JSX in each array factory below.
const RecoveryPhraseIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"
    />
  </svg>
);
const TeamIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z"
    />
  </svg>
);
const PrivacyIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z"
    />
  </svg>
);
const MigrationIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M7 16V4m0 0L3 8m4-4l4 4m6 0v12m0 0l4-4m-4 4l-4-4"
    />
  </svg>
);
const ScreenIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M3 5h18v12H3zM8 21h8m-4-4v4"
    />
  </svg>
);
const MessagingIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M8 10h.01M12 10h.01M16 10h.01M21 11c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 19l1.395-3.72C3.512 14.042 3 12.574 3 11c0-4.418 4.03-8 9-8s9 3.582 9 8z"
    />
  </svg>
);
const NotificationsIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
    />
  </svg>
);
const ToolsIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M10.325 4.317c.426-1.756 2.924-1.756 3.35 0a1.724 1.724 0 002.573 1.066c1.543-.94 3.31.826 2.37 2.37a1.724 1.724 0 001.066 2.573c1.756.426 1.756 2.924 0 3.35a1.724 1.724 0 00-1.066 2.573c.94 1.543-.826 3.31-2.37 2.37a1.724 1.724 0 00-2.573 1.066c-.426 1.756-2.924 1.756-3.35 0a1.724 1.724 0 00-2.573-1.066c-1.543.94-3.31-.826-2.37-2.37a1.724 1.724 0 00-1.066-2.573c-1.756-.426-1.756-2.924 0-3.35a1.724 1.724 0 001.066-2.573c-.94-1.543.826-3.31 2.37-2.37.996.608 2.296.07 2.572-1.065z"
    />
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M15 12a3 3 0 11-6 0 3 3 0 016 0z"
    />
  </svg>
);
const LlmIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"
    />
  </svg>
);
const CompanionIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M8 10h.01M12 10h.01M16 10h.01M9 16H5a2 2 0 01-2-2V6a2 2 0 012-2h14a2 2 0 012 2v8a2 2 0 01-2 2h-5l-5 5v-5z"
    />
  </svg>
);
const VoiceIcon = (
  <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path
      strokeLinecap="round"
      strokeLinejoin="round"
      strokeWidth={2}
      d="M19 11a7 7 0 01-7 7m0 0a7 7 0 01-7-7m7 7v4m0 0H8m4 0h4m-4-8a3 3 0 01-3-3V5a3 3 0 116 0v6a3 3 0 01-3 3z"
    />
  </svg>
);

const WrappedSettingsPage = ({
  children,
  maxWidthClass = 'max-w-lg',
}: {
  children: ReactNode;
  maxWidthClass?: string;
}) => {
  return (
    <div className="p-4 pt-6">
      <div
        className={`${maxWidthClass} mx-auto bg-white dark:bg-neutral-900 rounded-2xl shadow-soft border border-stone-200 dark:border-neutral-800 overflow-hidden`}>
        {children}
      </div>
    </div>
  );
};

const Settings = () => {
  const { t } = useT();

  const wrapSettingsPage = (element: ReactNode, opts?: { maxWidthClass?: string }) => (
    <WrappedSettingsPage maxWidthClass={opts?.maxWidthClass}>
      {element}
      <div className="border-t border-stone-100 dark:border-neutral-800 px-4 py-3 text-center text-[11px] text-stone-400 dark:text-neutral-500">
        {t('settings.betaBuild').replace('{version}', APP_VERSION)}
      </div>
    </WrappedSettingsPage>
  );

  const accountSettingsItems = [
    {
      id: 'recovery-phrase',
      title: t('pages.settings.account.recoveryPhrase'),
      description: t('pages.settings.account.recoveryPhraseDesc'),
      route: 'recovery-phrase',
      icon: RecoveryPhraseIcon,
    },
    {
      id: 'team',
      title: t('pages.settings.account.team'),
      description: t('pages.settings.account.teamDesc'),
      route: 'team',
      icon: TeamIcon,
    },
    {
      id: 'privacy',
      title: t('pages.settings.account.privacy'),
      description: t('pages.settings.account.privacyDesc'),
      route: 'privacy',
      icon: PrivacyIcon,
    },
    {
      id: 'migration',
      title: t('pages.settings.account.migration'),
      description: t('pages.settings.account.migrationDesc'),
      route: 'migration',
      icon: MigrationIcon,
    },
  ];

  const featuresSettingsItems = [
    {
      id: 'screen-intelligence',
      title: t('pages.settings.features.screenAwareness'),
      description: t('pages.settings.features.screenAwarenessDesc'),
      route: 'screen-intelligence',
      icon: ScreenIcon,
    },
    // Autocomplete + Voice Dictation hidden per #717 (routes retained for re-enable).
    {
      id: 'messaging',
      title: t('pages.settings.features.messagingChannels'),
      description: t('pages.settings.features.messagingChannelsDesc'),
      route: 'messaging',
      icon: MessagingIcon,
    },
    {
      id: 'notifications',
      title: t('pages.settings.features.notifications'),
      description: t('pages.settings.features.notificationsDesc'),
      route: 'notifications',
      icon: NotificationsIcon,
    },
    {
      id: 'tools',
      title: t('pages.settings.features.tools'),
      description: t('pages.settings.features.toolsDesc'),
      route: 'tools',
      icon: ToolsIcon,
    },
    {
      id: 'companion',
      title: t('pages.settings.features.desktopCompanion'),
      description: t('pages.settings.features.desktopCompanionDesc'),
      route: 'companion',
      icon: CompanionIcon,
    },
  ];

  const aiSettingsItems = [
    {
      id: 'llm',
      title: t('pages.settings.ai.llm'),
      description: t('pages.settings.ai.llmDesc'),
      route: 'llm',
      icon: LlmIcon,
    },
    {
      id: 'embeddings',
      title: t('pages.settings.ai.embeddings'),
      description: t('pages.settings.ai.embeddingsDesc'),
      route: 'embeddings',
      icon: LlmIcon,
    },
    {
      id: 'voice',
      title: t('pages.settings.ai.voice'),
      description: t('pages.settings.ai.voiceDesc'),
      route: 'voice',
      icon: VoiceIcon,
    },
    {
      id: 'agent-chat',
      title: t('settings.developerMenu.agentChat.title'),
      description: t('settings.developerMenu.agentChat.desc'),
      route: 'agent-chat',
      icon: LlmIcon,
    },
    {
      id: 'autonomy',
      title: t('settings.developerMenu.autonomy.title'),
      description: t('settings.developerMenu.autonomy.desc'),
      route: 'autonomy',
      icon: LlmIcon,
    },
    {
      id: 'local-model-debug',
      title: t('settings.developerMenu.localModelDebug.title'),
      description: t('settings.developerMenu.localModelDebug.desc'),
      route: 'local-model-debug',
      icon: LlmIcon,
    },
    {
      id: 'heartbeat',
      title: t('settings.heartbeat.title'),
      description: t('settings.heartbeat.desc'),
      route: 'heartbeat',
      icon: LlmIcon,
    },
    {
      id: 'ledger-usage',
      title: t('settings.ledgerUsage.title'),
      description: t('settings.ledgerUsage.desc'),
      route: 'ledger-usage',
      icon: LlmIcon,
    },
    {
      id: 'cost-dashboard',
      title: t('settings.costDashboard.title'),
      description: t('settings.costDashboard.desc'),
      route: 'cost-dashboard',
      icon: LlmIcon,
    },
  ];

  const composioSettingsItems = [
    {
      id: 'task-sources',
      title: t('settings.taskSources.title'),
      description: t('settings.taskSources.subtitle'),
      route: 'task-sources',
      icon: ToolsIcon,
    },
    {
      id: 'composio-routing',
      title: t('settings.developerMenu.composioRouting.title'),
      description: t('settings.developerMenu.composioRouting.desc'),
      route: 'composio-routing',
      icon: ToolsIcon,
    },
    {
      id: 'webhooks-triggers',
      title: t('settings.developerMenu.composeioTriggers.title'),
      description: t('settings.developerMenu.composeioTriggers.desc'),
      route: 'webhooks-triggers',
      icon: ToolsIcon,
    },
  ];

  return (
    <div>
      <Routes>
        <Route index element={wrapSettingsPage(<SettingsHome />)} />
        <Route
          path="account"
          element={wrapSettingsPage(
            <SettingsSectionPage
              title={t('pages.settings.accountSection.title')}
              description={t('pages.settings.accountSection.description')}
              items={accountSettingsItems}
              footer={<LogoutAndClearActions />}
            />
          )}
        />
        <Route
          path="features"
          element={wrapSettingsPage(
            <SettingsSectionPage
              title={t('pages.settings.featuresSection.title')}
              description={t('pages.settings.featuresSection.description')}
              items={featuresSettingsItems}
            />
          )}
        />
        <Route
          path="ai"
          element={wrapSettingsPage(
            <SettingsSectionPage
              title={t('pages.settings.aiSection.title')}
              description={t('pages.settings.aiSection.description')}
              items={aiSettingsItems}
            />
          )}
        />
        <Route
          path="composio"
          element={wrapSettingsPage(
            <SettingsSectionPage
              title={t('pages.settings.composioSection.title')}
              description={t('pages.settings.composioSection.description')}
              items={composioSettingsItems}
            />
          )}
        />
        {/* Account & Billing leaf panels */}
        <Route path="recovery-phrase" element={wrapSettingsPage(<RecoveryPhrasePanel />)} />
        <Route path="team" element={wrapSettingsPage(<TeamPanel />)} />
        <Route path="team/manage/:teamId" element={wrapSettingsPage(<TeamManagementPanel />)} />
        <Route
          path="team/manage/:teamId/members"
          element={wrapSettingsPage(<TeamMembersPanel />)}
        />
        <Route
          path="team/manage/:teamId/invites"
          element={wrapSettingsPage(<TeamInvitesPanel />)}
        />
        <Route path="team/members" element={wrapSettingsPage(<TeamMembersPanel />)} />
        <Route path="team/invites" element={wrapSettingsPage(<TeamInvitesPanel />)} />
        {/* BillingPanel intentionally uses its own wider layout. */}
        <Route path="billing" element={<BillingPanel />} />
        <Route path="privacy" element={wrapSettingsPage(<PrivacyPanel />)} />
        <Route path="migration" element={wrapSettingsPage(<MigrationPanel />)} />
        {/* Features leaf panels */}
        <Route path="screen-intelligence" element={wrapSettingsPage(<ScreenIntelligencePanel />)} />
        <Route path="autocomplete" element={wrapSettingsPage(<AutocompletePanel />)} />
        <Route path="voice" element={wrapSettingsPage(<VoicePanel />)} />
        <Route path="notifications" element={wrapSettingsPage(<NotificationsTabbedPanel />)} />
        <Route path="mascot" element={wrapSettingsPage(<MascotPanel />)} />
        <Route path="persona" element={wrapSettingsPage(<PersonaPanel />)} />
        <Route path="appearance" element={wrapSettingsPage(<AppearancePanel />)} />
        <Route path="agent-access" element={wrapSettingsPage(<AgentAccessPanel />)} />
        <Route path="agents" element={wrapSettingsPage(<AgentsPanel />)} />
        <Route path="tools" element={wrapSettingsPage(<ToolsPanel />)} />
        <Route path="companion" element={wrapSettingsPage(<CompanionPanel />)} />
        {/* Developer Options */}
        <Route path="developer-options" element={wrapSettingsPage(<DeveloperOptionsPanel />)} />
        <Route
          path="tool-policy-diagnostics"
          element={wrapSettingsPage(<ToolPolicyDiagnosticsPanel />)}
        />
        <Route path="autonomy" element={wrapSettingsPage(<AutonomyPanel />)} />
        <Route path="mcp-server" element={wrapSettingsPage(<McpServerPanel />)} />
        {/* Legacy direct path for the routing tab — kept so existing links
            (Developer Options entries, walkthroughs) keep working. The
            tabbed panel reads the URL hash to land on the right tab. */}
        <Route
          path="notification-routing"
          element={<Navigate to="/settings/notifications#routing" replace />}
        />
        <Route path="llm" element={wrapSettingsPage(<AIPanel />, { maxWidthClass: 'max-w-4xl' })} />
        <Route path="embeddings" element={wrapSettingsPage(<EmbeddingsPanel />)} />
        <Route
          path="heartbeat"
          element={wrapSettingsPage(<HeartbeatPanel />, { maxWidthClass: 'max-w-4xl' })}
        />
        <Route
          path="ledger-usage"
          element={wrapSettingsPage(<LedgerUsagePanel />, { maxWidthClass: 'max-w-4xl' })}
        />
        <Route
          path="cost-dashboard"
          element={wrapSettingsPage(<CostDashboardPanel />, { maxWidthClass: 'max-w-4xl' })}
        />
        <Route path="search" element={wrapSettingsPage(<SearchPanel />)} />
        <Route path="agent-chat" element={wrapSettingsPage(<AgentChatPanel />)} />
        <Route path="cron-jobs" element={wrapSettingsPage(<CronJobsPanel />)} />
        <Route path="task-sources" element={wrapSettingsPage(<TaskSourcesPanel />)} />
        <Route path="dev-workflow" element={wrapSettingsPage(<DevWorkflowPanel />)} />
        <Route path="skills-runner" element={wrapSettingsPage(<SkillsRunnerPanel />)} />
        <Route
          path="screen-awareness-debug"
          element={wrapSettingsPage(<ScreenAwarenessDebugPanel />)}
        />
        <Route path="autocomplete-debug" element={wrapSettingsPage(<AutocompleteDebugPanel />)} />
        <Route path="voice-debug" element={wrapSettingsPage(<VoiceDebugPanel />)} />
        <Route path="local-model-debug" element={wrapSettingsPage(<LocalModelDebugPanel />)} />
        <Route path="webhooks-debug" element={wrapSettingsPage(<WebhooksDebugPanel />)} />
        <Route path="event-log" element={wrapSettingsPage(<EventLogPanel />)} />
        <Route
          path="model-health"
          element={wrapSettingsPage(<ModelHealthPanel />, { maxWidthClass: 'max-w-4xl' })}
        />
        <Route path="memory-data" element={wrapSettingsPage(<MemoryDataPanel />)} />
        <Route path="memory-debug" element={wrapSettingsPage(<MemoryDebugPanel />)} />
        <Route path="intelligence" element={<Intelligence />} />
        <Route path="webhooks-triggers" element={<Webhooks />} />
        <Route path="composio-triggers" element={wrapSettingsPage(<ComposioTriagePanel />)} />
        <Route path="composio-routing" element={wrapSettingsPage(<ComposioPanel />)} />
        {/* Mobile devices */}
        <Route path="devices" element={wrapSettingsPage(<DevicesComingSoonPanel />)} />
        {/* About / updates */}
        <Route path="about" element={wrapSettingsPage(<AboutPanel />)} />
        {/* Fallback */}
        <Route path="*" element={<Navigate to="/settings" replace />} />
      </Routes>
    </div>
  );
};

export default Settings;
