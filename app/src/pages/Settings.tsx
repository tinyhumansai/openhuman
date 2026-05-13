import type { ReactNode } from 'react';
import { Navigate, Route, Routes } from 'react-router-dom';

import AboutPanel from '../components/settings/panels/AboutPanel';
import AgentChatPanel from '../components/settings/panels/AgentChatPanel';
import AIPanel from '../components/settings/panels/AIPanel';
import AutocompleteDebugPanel from '../components/settings/panels/AutocompleteDebugPanel';
import AutocompletePanel from '../components/settings/panels/AutocompletePanel';
import BackendProviderPanel from '../components/settings/panels/BackendProviderPanel';
import BillingPanel from '../components/settings/panels/BillingPanel';
import ComposioTriagePanel from '../components/settings/panels/ComposioTriagePanel';
import ConnectionsPanel from '../components/settings/panels/ConnectionsPanel';
import CronJobsPanel from '../components/settings/panels/CronJobsPanel';
import DeveloperOptionsPanel from '../components/settings/panels/DeveloperOptionsPanel';
import LocalModelDebugPanel from '../components/settings/panels/LocalModelDebugPanel';
import LocalModelPanel from '../components/settings/panels/LocalModelPanel';
import MemoryDataPanel from '../components/settings/panels/MemoryDataPanel';
import MemoryDebugPanel from '../components/settings/panels/MemoryDebugPanel';
import MessagingPanel from '../components/settings/panels/MessagingPanel';
import NotificationRoutingPanel from '../components/settings/panels/NotificationRoutingPanel';
import NotificationsPanel from '../components/settings/panels/NotificationsPanel';
import PrivacyPanel from '../components/settings/panels/PrivacyPanel';
import RecoveryPhrasePanel from '../components/settings/panels/RecoveryPhrasePanel';
import ScreenAwarenessDebugPanel from '../components/settings/panels/ScreenAwarenessDebugPanel';
import ScreenIntelligencePanel from '../components/settings/panels/ScreenIntelligencePanel';
import TeamInvitesPanel from '../components/settings/panels/TeamInvitesPanel';
import TeamManagementPanel from '../components/settings/panels/TeamManagementPanel';
import TeamMembersPanel from '../components/settings/panels/TeamMembersPanel';
import TeamPanel from '../components/settings/panels/TeamPanel';
import ToolsPanel from '../components/settings/panels/ToolsPanel';
import VoiceDebugPanel from '../components/settings/panels/VoiceDebugPanel';
import VoicePanel from '../components/settings/panels/VoicePanel';
import WebhooksDebugPanel from '../components/settings/panels/WebhooksDebugPanel';
import SettingsHome from '../components/settings/SettingsHome';
import SettingsSectionPage from '../components/settings/SettingsSectionPage';
import { APP_VERSION } from '../utils/config';
import Intelligence from './Intelligence';
import Webhooks from './Webhooks';

const accountSettingsItems = [
  {
    id: 'recovery-phrase',
    title: 'Recovery Phrase',
    description: 'Manage your BIP39 recovery phrase for encryption and wallet access',
    route: 'recovery-phrase',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M15 7a2 2 0 012 2m4 0a6 6 0 01-7.743 5.743L11 17H9v2H7v2H4a1 1 0 01-1-1v-2.586a1 1 0 01.293-.707l5.964-5.964A6 6 0 1121 9z"
        />
      </svg>
    ),
  },
  {
    id: 'team',
    title: 'Team',
    description: 'Manage your team, members, and invites',
    route: 'team',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M17 20h5v-2a3 3 0 00-5.356-1.857M17 20H7m10 0v-2c0-.656-.126-1.283-.356-1.857M7 20H2v-2a3 3 0 015.356-1.857M7 20v-2c0-.656.126-1.283.356-1.857m0 0a5.002 5.002 0 019.288 0M15 7a3 3 0 11-6 0 3 3 0 016 0zm6 3a2 2 0 11-4 0 2 2 0 014 0zM7 10a2 2 0 11-4 0 2 2 0 014 0z"
        />
      </svg>
    ),
  },
  {
    id: 'connections',
    title: 'Connections',
    description: 'Review and manage linked account connections',
    route: 'connections',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M13.828 10.172a4 4 0 010 5.656l-2 2a4 4 0 01-5.656-5.656l1-1m5-5a4 4 0 015.656 5.656l-1 1m-5 5l5-5"
        />
      </svg>
    ),
  },
  {
    id: 'privacy',
    title: 'Privacy',
    description: 'Manage data sharing and anonymized usage preferences',
    route: 'privacy',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9 12l2 2 4-4m5.618-4.016A11.955 11.955 0 0112 2.944a11.955 11.955 0 01-8.618 3.04A12.02 12.02 0 003 9c0 5.591 3.824 10.29 9 11.622 5.176-1.332 9-6.03 9-11.622 0-1.042-.133-2.052-.382-3.016z"
        />
      </svg>
    ),
  },
];

const featuresSettingsItems = [
  {
    id: 'screen-intelligence',
    title: 'Screen Awareness',
    description: 'Screen capture permissions, monitoring policy, and session controls',
    route: 'screen-intelligence',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M3 5h18v12H3zM8 21h8m-4-4v4"
        />
      </svg>
    ),
  },
  // Autocomplete + Voice Dictation hidden per #717 (routes retained for re-enable).
  {
    id: 'messaging',
    title: 'Messaging Channels',
    description: 'Configure Telegram/Discord auth modes and default channel routing',
    route: 'messaging',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M8 10h.01M12 10h.01M16 10h.01M21 11c0 4.418-4.03 8-9 8a9.863 9.863 0 01-4.255-.949L3 19l1.395-3.72C3.512 14.042 3 12.574 3 11c0-4.418 4.03-8 9-8s9 3.582 9 8z"
        />
      </svg>
    ),
  },
  {
    id: 'notifications',
    title: 'Notifications',
    description: 'Choose which categories surface in the notification center',
    route: 'notifications',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M15 17h5l-1.405-1.405A2.032 2.032 0 0118 14.158V11a6.002 6.002 0 00-4-5.659V5a2 2 0 10-4 0v.341C7.67 6.165 6 8.388 6 11v3.159c0 .538-.214 1.055-.595 1.436L4 17h5m6 0v1a3 3 0 11-6 0v-1m6 0H9"
        />
      </svg>
    ),
  },
  {
    id: 'tools',
    title: 'Tools',
    description: 'Enable or disable capabilities OpenHuman can use on your behalf',
    route: 'tools',
    icon: (
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
    ),
  },
];

const aiModelsSettingsItems = [
  {
    id: 'local-model',
    title: 'Local AI Model',
    description: 'Choose model tier by device capability and manage downloads',
    route: 'local-model',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9 3v2m6-2v2M9 19v2m6-2v2M5 9H3m2 6H3m18-6h-2m2 6h-2M7 19h10a2 2 0 002-2V7a2 2 0 00-2-2H7a2 2 0 00-2 2v10a2 2 0 002 2zM9 9h6v6H9V9z"
        />
      </svg>
    ),
  },
  {
    id: 'backend-provider',
    title: 'LLM Provider',
    description: 'Point inference at the OpenHuman backend or any OpenAI-compatible provider',
    route: 'backend-provider',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M4 7a2 2 0 012-2h12a2 2 0 012 2v4a2 2 0 01-2 2H6a2 2 0 01-2-2V7zm0 10a2 2 0 012-2h12a2 2 0 012 2v0a2 2 0 01-2 2H6a2 2 0 01-2-2v0zM7 9h.01M7 17h.01"
        />
      </svg>
    ),
  },
];

const WrappedSettingsPage = ({ children }: { children: ReactNode }) => {
  return (
    <div className="p-4 pt-6">
      <div className="max-w-lg mx-auto bg-white rounded-2xl shadow-soft border border-stone-200 overflow-hidden">
        {children}
      </div>
    </div>
  );
};

function wrapSettingsPage(element: ReactNode) {
  return (
    <WrappedSettingsPage>
      {element}
      <div className="border-t border-stone-100 px-4 py-3 text-center text-[11px] text-stone-400">
        Beta build - v{APP_VERSION}
      </div>
    </WrappedSettingsPage>
  );
}

const Settings = () => {
  return (
    <div>
      <Routes>
        <Route index element={wrapSettingsPage(<SettingsHome />)} />
        <Route
          path="account"
          element={wrapSettingsPage(
            <SettingsSectionPage
              title="Account"
              description="Recovery phrase, team, connections, and privacy settings."
              items={accountSettingsItems}
            />
          )}
        />
        <Route
          path="features"
          element={wrapSettingsPage(
            <SettingsSectionPage
              title="Features"
              description="Screen awareness, messaging, and tools."
              items={featuresSettingsItems}
            />
          )}
        />
        <Route
          path="ai-models"
          element={wrapSettingsPage(
            <SettingsSectionPage
              title="AI & Models"
              description="Local AI model setup and backend provider configuration."
              items={aiModelsSettingsItems}
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
        <Route path="connections" element={wrapSettingsPage(<ConnectionsPanel />)} />
        {/* BillingPanel intentionally uses its own wider layout. */}
        <Route path="billing" element={<BillingPanel />} />
        <Route path="privacy" element={wrapSettingsPage(<PrivacyPanel />)} />
        {/* Features leaf panels */}
        <Route path="screen-intelligence" element={wrapSettingsPage(<ScreenIntelligencePanel />)} />
        <Route path="autocomplete" element={wrapSettingsPage(<AutocompletePanel />)} />
        <Route path="voice" element={wrapSettingsPage(<VoicePanel />)} />
        <Route path="messaging" element={wrapSettingsPage(<MessagingPanel />)} />
        <Route path="notifications" element={wrapSettingsPage(<NotificationsPanel />)} />
        <Route path="tools" element={wrapSettingsPage(<ToolsPanel />)} />
        {/* AI & Models leaf panels */}
        <Route path="local-model" element={wrapSettingsPage(<LocalModelPanel />)} />
        <Route path="backend-provider" element={wrapSettingsPage(<BackendProviderPanel />)} />
        {/* Developer Options */}
        <Route path="developer-options" element={wrapSettingsPage(<DeveloperOptionsPanel />)} />
        <Route
          path="notification-routing"
          element={wrapSettingsPage(<NotificationRoutingPanel />)}
        />
        <Route path="ai" element={wrapSettingsPage(<AIPanel />)} />
        <Route path="agent-chat" element={wrapSettingsPage(<AgentChatPanel />)} />
        <Route path="cron-jobs" element={wrapSettingsPage(<CronJobsPanel />)} />
        <Route
          path="screen-awareness-debug"
          element={wrapSettingsPage(<ScreenAwarenessDebugPanel />)}
        />
        <Route path="autocomplete-debug" element={wrapSettingsPage(<AutocompleteDebugPanel />)} />
        <Route path="voice-debug" element={wrapSettingsPage(<VoiceDebugPanel />)} />
        <Route path="local-model-debug" element={wrapSettingsPage(<LocalModelDebugPanel />)} />
        <Route path="webhooks-debug" element={wrapSettingsPage(<WebhooksDebugPanel />)} />
        <Route path="memory-data" element={wrapSettingsPage(<MemoryDataPanel />)} />
        <Route path="memory-debug" element={wrapSettingsPage(<MemoryDebugPanel />)} />
        <Route path="intelligence" element={<Intelligence />} />
        <Route path="webhooks-triggers" element={<Webhooks />} />
        <Route path="composio-triggers" element={wrapSettingsPage(<ComposioTriagePanel />)} />
        {/* About / updates */}
        <Route path="about" element={wrapSettingsPage(<AboutPanel />)} />
        {/* Fallback */}
        <Route path="*" element={<Navigate to="/settings" replace />} />
      </Routes>
    </div>
  );
};

export default Settings;
