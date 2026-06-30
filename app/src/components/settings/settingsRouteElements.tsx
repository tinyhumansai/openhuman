import type { ReactNode } from 'react';
import { Navigate, Route, useLocation } from 'react-router-dom';

import WorkflowsTab from '../intelligence/WorkflowsTab';
import SettingsIndexRedirect from './layout/SettingsIndexRedirect';
import AboutPanel from './panels/AboutPanel';
import AccountPanel from './panels/AccountPanel';
import AgentAccessPanel from './panels/AgentAccessPanel';
import AgentActivityPanel from './panels/AgentActivityPanel';
import AgentBoxPanel from './panels/AgentBoxPanel';
import AgentChatPanel from './panels/AgentChatPanel';
import AgentEditorPage from './panels/AgentEditorPage';
import AgentsPanel from './panels/AgentsPanel';
import AppearancePanel from './panels/AppearancePanel';
import ApprovalHistoryPanel from './panels/ApprovalHistoryPanel';
import AutocompleteDebugPanel from './panels/AutocompleteDebugPanel';
import AutocompletePanel from './panels/AutocompletePanel';
import BillingPanel from './panels/BillingPanel';
import CompanionPanel from './panels/CompanionPanel';
import ComposioTriagePanel from './panels/ComposioTriagePanel';
import CronJobsPanel from './panels/CronJobsPanel';
import DesktopAgentPanel from './panels/DesktopAgentPanel';
import DeveloperOptionsPanel from './panels/DeveloperOptionsPanel';
import DevicesPanel from './panels/DevicesPanel';
import DevWorkflowPanel from './panels/DevWorkflowPanel';
import EventLogPanel from './panels/EventLogPanel';
import IntegrationsPanel from './panels/IntegrationsPanel';
import KeyboardShortcutsPanel from './panels/KeyboardShortcutsPanel';
import LocalModelDebugPanel from './panels/LocalModelDebugPanel';
import McpServerPanel from './panels/McpServerPanel';
import MeetingSettingsPanel from './panels/MeetingSettingsPanel';
import MemorySyncPanel from './panels/MemorySyncPanel';
import MigrationPanel from './panels/MigrationPanel';
import ModelHealthPanel from './panels/ModelHealthPanel';
import NotificationsTabbedPanel from './panels/NotificationsTabbedPanel';
import PermissionsPanel from './panels/PermissionsPanel';
import PersonalityPanel from './panels/PersonalityPanel';
import PrivacyPanel from './panels/PrivacyPanel';
import ProfileEditorPage from './panels/ProfileEditorPage';
import ProfilesPanel from './panels/ProfilesPanel';
import RecoveryPhrasePanel from './panels/RecoveryPhrasePanel';
import SandboxSettingsPanel from './panels/SandboxSettingsPanel';
import ScreenAwarenessDebugPanel from './panels/ScreenAwarenessDebugPanel';
import ScreenIntelligencePanel from './panels/ScreenIntelligencePanel';
import SecurityPanel from './panels/SecurityPanel';
import TasksPanel from './panels/TasksPanel';
import TeamInvitesPanel from './panels/TeamInvitesPanel';
import TeamManagementPanel from './panels/TeamManagementPanel';
import TeamMembersPanel from './panels/TeamMembersPanel';
import TeamPanel from './panels/TeamPanel';
import ThemeStudioPanel from './panels/ThemeStudioPanel';
import TokenUsagePanel from './panels/TokenUsagePanel';
import ToolPolicyDiagnosticsPanel from './panels/ToolPolicyDiagnosticsPanel';
import ToolsPanel from './panels/ToolsPanel';
import UsagePanel from './panels/UsagePanel';
import VoiceDebugPanel from './panels/VoiceDebugPanel';
import WalletBalancesPanel from './panels/WalletBalancesPanel';
import WebhooksDebugPanel from './panels/WebhooksDebugPanel';
import WorkflowRunnerPanel from './panels/WorkflowRunnerPanel';

/**
 * Single vertical-scroll wrapper for a settings panel. The surrounding card
 * (bg / border / rounding) is provided by the host — `SettingsLayout`'s content
 * pane on iOS, or `SettingsModalLayout`'s right column on desktop — so panels
 * sit directly on it. PanelScaffold-based panels are `h-full` and own their own
 * internal scroll; legacy panels that overflow scroll here. Either way there's
 * exactly one scrollbar.
 */
export const WrappedSettingsPage = ({ children }: { children: ReactNode }) => {
  return <div className="h-full min-h-0 overflow-y-auto">{children}</div>;
};

const wrapSettingsPage = (element: ReactNode) => (
  <WrappedSettingsPage>{element}</WrappedSettingsPage>
);

/**
 * Redirect that stays *within* `/settings/*` while preserving nav state — most
 * importantly the desktop modal's `backgroundLocation`, so a legacy-slug hop
 * inside the modal keeps its backdrop instead of falling back to the default
 * page. Use this for in-settings redirects only; external redirects (`/brain`,
 * `/connections`) intentionally exit the modal and keep a plain `<Navigate>`.
 */
const SettingsRedirect = ({ to }: { to: string }) => {
  const location = useLocation();
  return <Navigate to={to} replace state={location.state} />;
};

/**
 * The full settings route table — index, every panel, and every legacy-slug
 * redirect. Returned as a fragment of `<Route>` elements (via a function call,
 * not a nested component) so it can be embedded directly inside a `<Routes>` in
 * both hosts:
 *
 *   - Desktop modal: `<Routes>{settingsRouteElements()}</Routes>`
 *   - iOS full page:  `<Routes><Route element={<SettingsLayout/>}>{settingsRouteElements()}</Route></Routes>`
 *
 * Retired slugs are kept as redirects so deep links keep working.
 */
export function settingsRouteElements(): ReactNode {
  return (
    <>
      <Route index element={<SettingsIndexRedirect />} />

      {/* ── General ─────────────────────────────────────────────── */}
      <Route path="account" element={wrapSettingsPage(<AccountPanel />)} />
      <Route path="team" element={wrapSettingsPage(<TeamPanel />)} />
      <Route path="team/manage/:teamId" element={wrapSettingsPage(<TeamManagementPanel />)} />
      <Route path="team/manage/:teamId/members" element={wrapSettingsPage(<TeamMembersPanel />)} />
      <Route path="team/manage/:teamId/invites" element={wrapSettingsPage(<TeamInvitesPanel />)} />
      <Route path="team/members" element={wrapSettingsPage(<TeamMembersPanel />)} />
      <Route path="team/invites" element={wrapSettingsPage(<TeamInvitesPanel />)} />
      <Route path="billing" element={wrapSettingsPage(<BillingPanel />)} />
      <Route path="privacy" element={wrapSettingsPage(<PrivacyPanel />)} />
      <Route path="security" element={wrapSettingsPage(<SecurityPanel />)} />
      <Route path="migration" element={wrapSettingsPage(<MigrationPanel />)} />
      <Route path="appearance" element={wrapSettingsPage(<AppearancePanel />)} />
      <Route path="theme" element={wrapSettingsPage(<ThemeStudioPanel />)} />
      <Route path="notifications" element={wrapSettingsPage(<NotificationsTabbedPanel />)} />
      {/* Real device-pairing panel (replaces the old "Coming Soon" stub). */}
      <Route path="devices" element={wrapSettingsPage(<DevicesPanel />)} />

      {/* ── Assistant ───────────────────────────────────────────── */}
      {/* LLM / Voice / Embeddings moved to the Connections page. */}
      <Route path="llm" element={<Navigate to="/connections?tab=llm" replace />} />
      <Route path="embeddings" element={<Navigate to="/connections?tab=embeddings" replace />} />
      <Route path="usage" element={wrapSettingsPage(<UsagePanel />)} />
      <Route path="voice" element={<Navigate to="/connections?tab=voice" replace />} />
      <Route path="personality" element={wrapSettingsPage(<PersonalityPanel />)} />
      <Route path="agents" element={wrapSettingsPage(<AgentsPanel />)} />
      <Route path="agents/new" element={wrapSettingsPage(<AgentEditorPage />)} />
      <Route path="agents/edit/:id" element={wrapSettingsPage(<AgentEditorPage />)} />
      {/* Top-level agent profiles (soul, memory, skills, MCP, connectors). */}
      <Route path="profiles" element={wrapSettingsPage(<ProfilesPanel />)} />
      <Route path="profiles/new" element={wrapSettingsPage(<ProfileEditorPage />)} />
      <Route path="profiles/edit/:id" element={wrapSettingsPage(<ProfileEditorPage />)} />
      <Route path="agent-access" element={wrapSettingsPage(<AgentAccessPanel />)} />
      <Route path="activity-level" element={wrapSettingsPage(<AgentActivityPanel />)} />
      <Route path="sandbox-settings" element={wrapSettingsPage(<SandboxSettingsPanel />)} />
      <Route path="approval-history" element={wrapSettingsPage(<ApprovalHistoryPanel />)} />

      {/* ── Data ────────────────────────────────────────────────── */}
      <Route path="memory-sync" element={wrapSettingsPage(<MemorySyncPanel />)} />
      <Route path="wallet-balances" element={wrapSettingsPage(<WalletBalancesPanel />)} />
      <Route path="recovery-phrase" element={wrapSettingsPage(<RecoveryPhrasePanel />)} />

      {/* ── Connections ─────────────────────────────────────────── */}
      <Route path="integrations" element={wrapSettingsPage(<IntegrationsPanel />)} />
      <Route path="screen-intelligence" element={wrapSettingsPage(<ScreenIntelligencePanel />)} />
      <Route path="desktop-agent" element={wrapSettingsPage(<DesktopAgentPanel />)} />
      <Route path="tools" element={wrapSettingsPage(<ToolsPanel />)} />
      <Route path="companion" element={wrapSettingsPage(<CompanionPanel />)} />
      <Route path="meetings" element={wrapSettingsPage(<MeetingSettingsPanel />)} />
      <Route path="autocomplete" element={wrapSettingsPage(<AutocompletePanel />)} />

      {/* ── System ──────────────────────────────────────────────── */}
      <Route path="keyboard-shortcuts" element={wrapSettingsPage(<KeyboardShortcutsPanel />)} />
      <Route path="developer-options" element={wrapSettingsPage(<DeveloperOptionsPanel />)} />
      <Route path="token-usage" element={wrapSettingsPage(<TokenUsagePanel />)} />
      <Route path="about" element={wrapSettingsPage(<AboutPanel />)} />

      {/* ── Developer & Diagnostics leaf panels ─────────────────── */}
      <Route
        path="tool-policy-diagnostics"
        element={wrapSettingsPage(<ToolPolicyDiagnosticsPanel />)}
      />
      <Route path="agentbox" element={wrapSettingsPage(<AgentBoxPanel />)} />
      <Route path="mcp-server" element={wrapSettingsPage(<McpServerPanel />)} />
      {/* Search engine settings moved to the Connections page. */}
      <Route path="search" element={<Navigate to="/connections?tab=search" replace />} />
      <Route path="agent-chat" element={wrapSettingsPage(<AgentChatPanel />)} />
      <Route path="cron-jobs" element={wrapSettingsPage(<CronJobsPanel />)} />
      <Route path="tasks" element={wrapSettingsPage(<TasksPanel />)} />
      <Route path="automations" element={wrapSettingsPage(<WorkflowsTab asSettingsPanel />)} />
      <Route path="dev-workflow" element={wrapSettingsPage(<DevWorkflowPanel />)} />
      <Route path="skills-runner" element={wrapSettingsPage(<WorkflowRunnerPanel />)} />
      <Route
        path="screen-awareness-debug"
        element={wrapSettingsPage(<ScreenAwarenessDebugPanel />)}
      />
      <Route path="autocomplete-debug" element={wrapSettingsPage(<AutocompleteDebugPanel />)} />
      <Route path="voice-debug" element={wrapSettingsPage(<VoiceDebugPanel />)} />
      <Route path="local-model-debug" element={wrapSettingsPage(<LocalModelDebugPanel />)} />
      <Route path="webhooks-debug" element={wrapSettingsPage(<WebhooksDebugPanel />)} />
      <Route path="event-log" element={wrapSettingsPage(<EventLogPanel />)} />
      <Route path="model-health" element={wrapSettingsPage(<ModelHealthPanel />)} />
      {/* Knowledge & Memory panels moved to the Brain page. */}
      <Route path="memory-data" element={<Navigate to="/brain?tab=memory-data" replace />} />
      <Route path="memory-debug" element={<Navigate to="/brain?tab=memory-debug" replace />} />
      <Route path="analysis-views" element={<Navigate to="/brain?tab=analysis-views" replace />} />
      <Route path="intelligence" element={<Navigate to="/brain?tab=intelligence" replace />} />
      <Route path="composio-triggers" element={wrapSettingsPage(<ComposioTriagePanel />)} />
      <Route path="permissions" element={wrapSettingsPage(<PermissionsPanel />)} />

      {/* ── Legacy slugs → redirects (deep-link compatibility) ──── */}
      {/* Old hub pages */}
      <Route path="ai" element={<Navigate to="/connections?tab=llm" replace />} />
      <Route path="agents-settings" element={<SettingsRedirect to="/settings/agents" />} />
      <Route path="features" element={<SettingsRedirect to="/settings/screen-intelligence" />} />
      <Route path="crypto" element={<SettingsRedirect to="/settings/wallet-balances" />} />
      <Route path="notifications-hub" element={<SettingsRedirect to="/settings/notifications" />} />
      {/* Composio (API key + routing) moved to Connections → API keys. */}
      <Route path="composio" element={<Navigate to="/connections?tab=composio-key" replace />} />
      {/* Merged Usage & Limits page */}
      <Route path="heartbeat" element={<SettingsRedirect to="/settings/usage#background" />} />
      <Route path="ledger-usage" element={<SettingsRedirect to="/settings/usage#background" />} />
      <Route path="cost-dashboard" element={<SettingsRedirect to="/settings/usage" />} />
      {/* Autonomy rate-limit lives inside Agent access now */}
      <Route path="autonomy" element={<SettingsRedirect to="/settings/agent-access" />} />
      {/* Merged Personality & Face page */}
      <Route path="mascot" element={<SettingsRedirect to="/settings/personality#face" />} />
      <Route path="persona" element={<SettingsRedirect to="/settings/personality" />} />
      {/* Merged Integrations page */}
      <Route path="task-sources" element={<SettingsRedirect to="/settings/integrations" />} />
      <Route
        path="composio-routing"
        element={<Navigate to="/connections?tab=composio-key" replace />}
      />
      <Route
        path="webhooks-triggers"
        element={<SettingsRedirect to="/settings/integrations#webhooks" />}
      />
      {/* Notification routing tab */}
      <Route
        path="notification-routing"
        element={<SettingsRedirect to="/settings/notifications#routing" />}
      />
      {/* Fallback */}
      <Route path="*" element={<SettingsRedirect to="/settings" />} />
    </>
  );
}
