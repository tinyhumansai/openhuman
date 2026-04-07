import { Route, Routes } from 'react-router-dom';

import AccessibilityPanel from '../components/settings/panels/AccessibilityPanel';
import AdvancedPanel from '../components/settings/panels/AdvancedPanel';
import AgentChatPanel from '../components/settings/panels/AgentChatPanel';
import AIPanel from '../components/settings/panels/AIPanel';
import AutocompletePanel from '../components/settings/panels/AutocompletePanel';
import BillingPanel from '../components/settings/panels/BillingPanel';
import ConnectionsPanel from '../components/settings/panels/ConnectionsPanel';
import CronJobsPanel from '../components/settings/panels/CronJobsPanel';
import DeveloperOptionsPanel from '../components/settings/panels/DeveloperOptionsPanel';
import LocalModelPanel from '../components/settings/panels/LocalModelPanel';
import MemoryDataPanel from '../components/settings/panels/MemoryDataPanel';
import MemoryDebugPanel from '../components/settings/panels/MemoryDebugPanel';
import MessagingPanel from '../components/settings/panels/MessagingPanel';
import PrivacyPanel from '../components/settings/panels/PrivacyPanel';
import ProfilePanel from '../components/settings/panels/ProfilePanel';
import RecoveryPhrasePanel from '../components/settings/panels/RecoveryPhrasePanel';
import ScreenIntelligencePanel from '../components/settings/panels/ScreenIntelligencePanel';
import SkillsPanel from '../components/settings/panels/SkillsPanel';
import TeamInvitesPanel from '../components/settings/panels/TeamInvitesPanel';
import TeamManagementPanel from '../components/settings/panels/TeamManagementPanel';
import TeamMembersPanel from '../components/settings/panels/TeamMembersPanel';
import TeamPanel from '../components/settings/panels/TeamPanel';
import ToolsPanel from '../components/settings/panels/ToolsPanel';
import VoicePanel from '../components/settings/panels/VoicePanel';
import WebhooksDebugPanel from '../components/settings/panels/WebhooksDebugPanel';
import SettingsHome from '../components/settings/SettingsHome';
import SettingsSectionPage from '../components/settings/SettingsSectionPage';

const accountSettingsItems = [
  {
    id: 'billing',
    title: 'Billing & Usage',
    description: 'Manage your subscription, credits, and payment methods',
    route: 'billing',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M3 10h18M7 15h1m4 0h1m-7 4h12a3 3 0 003-3V8a3 3 0 00-3 3v8a3 3 0 003 3z"
        />
      </svg>
    ),
  },
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
];

const automationSettingsItems = [
  {
    id: 'accessibility',
    title: 'Accessibility Automation',
    description: 'Desktop permissions, assisted controls, and safety-bound sessions',
    route: 'accessibility',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9 12h6m-7 9h8a2 2 0 002-2V7a2 2 0 00-2-2h-1l-.707-.707A1 1 0 0013.586 4h-3.172a1 1 0 00-.707.293L9 5H8a2 2 0 00-2 2v12a2 2 0 002 2z"
        />
      </svg>
    ),
  },
  {
    id: 'screen-intelligence',
    title: 'Screen Intelligence',
    description: 'Window capture policy, vision summaries, and memory ingestion',
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
  {
    id: 'autocomplete',
    title: 'Inline Autocomplete',
    description: 'Manage predictive text style, app filters, and live completion controls',
    route: 'autocomplete',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M4 7h16M4 12h10m-10 5h7m10 0l3 3m0 0l3-3m-3 3v-8"
        />
      </svg>
    ),
  },
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
    id: 'cron-jobs',
    title: 'Cron Jobs',
    description: 'View and configure scheduled jobs for runtime skills',
    route: 'cron-jobs',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M12 8v4l3 3m6-3a9 9 0 11-18 0 9 9 0 0118 0z"
        />
      </svg>
    ),
  },
];

const aiSettingsItems = [
  {
    id: 'voice',
    title: 'Voice Dictation',
    description: 'Manage dictation startup, hotkeys, writing style, and runtime controls',
    route: 'voice',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M12 3a3 3 0 00-3 3v6a3 3 0 006 0V6a3 3 0 00-3-3zm-7 9a7 7 0 0014 0m-7 7v2m-4 0h8"
        />
      </svg>
    ),
  },
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
    id: 'ai',
    title: 'AI Configuration',
    description: 'Configure persona, prompting behavior, and AI runtime settings',
    route: 'ai',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M12 3l1.9 3.85 4.25.62-3.08 3 .73 4.23L12 12.77 8.2 14.7l.73-4.23-3.08-3 4.25-.62L12 3z"
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
  {
    id: 'skills',
    title: 'Skills',
    description: 'Configure browser access, skill behavior, and installed skill capabilities',
    route: 'skills',
    icon: (
      <svg className="w-5 h-5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
        <path
          strokeLinecap="round"
          strokeLinejoin="round"
          strokeWidth={2}
          d="M9.75 3a.75.75 0 00-1.5 0v2.25H6a2.25 2.25 0 000 4.5h2.25V12H6a2.25 2.25 0 000 4.5h2.25V18a.75.75 0 001.5 0v-1.5H12V18a.75.75 0 001.5 0v-1.5H18a2.25 2.25 0 000-4.5h-4.5V9.75H18a2.25 2.25 0 000-4.5h-4.5V3a.75.75 0 00-1.5 0v2.25H9.75V3z"
        />
      </svg>
    ),
  },
];

const Settings = () => {
  return (
    <div className="p-4 pt-6">
      <div className="max-w-lg mx-auto bg-white rounded-2xl shadow-soft border border-stone-200 overflow-hidden">
        <Routes>
          <Route index element={<SettingsHome />} />
          <Route
            path="account"
            element={
              <SettingsSectionPage
                title="Account & Security"
                description="Billing, recovery, team access, and linked account settings."
                items={accountSettingsItems}
              />
            }
          />
          <Route
            path="automation"
            element={
              <SettingsSectionPage
                title="Automation & Channels"
                description="Desktop automation, capture, messaging, and scheduled jobs."
                items={automationSettingsItems}
              />
            }
          />
          <Route
            path="ai-tools"
            element={
              <SettingsSectionPage
                title="AI & Skills"
                description="Model management, AI behavior, and skill configuration."
                items={aiSettingsItems}
              />
            }
          />
          <Route path="connections" element={<ConnectionsPanel />} />
          <Route path="messaging" element={<MessagingPanel />} />
          <Route path="cron-jobs" element={<CronJobsPanel />} />
          <Route path="screen-intelligence" element={<ScreenIntelligencePanel />} />
          <Route path="autocomplete" element={<AutocompletePanel />} />
          <Route path="privacy" element={<PrivacyPanel />} />
          <Route path="profile" element={<ProfilePanel />} />
          <Route path="advanced" element={<AdvancedPanel />} />
          <Route path="agent-chat" element={<AgentChatPanel />} />
          <Route path="ai" element={<AIPanel />} />
          <Route path="accessibility" element={<AccessibilityPanel />} />
          <Route path="local-model" element={<LocalModelPanel />} />
          <Route path="voice" element={<VoicePanel />} />
          <Route path="billing" element={<BillingPanel />} />
          <Route path="skills" element={<SkillsPanel />} />
          <Route path="tools" element={<ToolsPanel />} />
          <Route path="team" element={<TeamPanel />} />
          <Route path="team/manage/:teamId" element={<TeamManagementPanel />} />
          <Route path="team/manage/:teamId/members" element={<TeamMembersPanel />} />
          <Route path="team/manage/:teamId/invites" element={<TeamInvitesPanel />} />
          <Route path="team/members" element={<TeamMembersPanel />} />
          <Route path="team/invites" element={<TeamInvitesPanel />} />
          <Route path="developer-options" element={<DeveloperOptionsPanel />} />
          <Route path="webhooks-debug" element={<WebhooksDebugPanel />} />
          <Route path="memory-data" element={<MemoryDataPanel />} />
          <Route path="memory-debug" element={<MemoryDebugPanel />} />
          <Route path="recovery-phrase" element={<RecoveryPhrasePanel />} />
        </Routes>
      </div>
    </div>
  );
};

export default Settings;
