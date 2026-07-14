import type { ReactNode } from 'react';
import { Navigate, Route, useLocation } from 'react-router-dom';

import SettingsIndexRedirect from './layout/SettingsIndexRedirect';
import AboutPanel from './panels/AboutPanel';
import AccountPanel from './panels/AccountPanel';
import AgentAccessPanel from './panels/AgentAccessPanel';
import AgentActivityPanel from './panels/AgentActivityPanel';
import AgentBoxPanel from './panels/AgentBoxPanel';
import AgentEditorPage from './panels/AgentEditorPage';
import AgentsPanel from './panels/AgentsPanel';
import AppearancePanel from './panels/AppearancePanel';
import ApprovalHistoryPanel from './panels/ApprovalHistoryPanel';
import AutocompleteDebugPanel from './panels/AutocompleteDebugPanel';
import AutocompletePanel from './panels/AutocompletePanel';
import BillingPanel from './panels/BillingPanel';
import CoreConnectionPanel from './panels/CoreConnectionPanel';
import CronJobsPanel from './panels/CronJobsPanel';
import DeveloperOptionsPanel from './panels/DeveloperOptionsPanel';
import DevicesPanel from './panels/DevicesPanel';
import EventLogPanel from './panels/EventLogPanel';
import KeyboardShortcutsPanel from './panels/KeyboardShortcutsPanel';
import McpServerPanel from './panels/McpServerPanel';
import MemoryDataPanel from './panels/MemoryDataPanel';
import MemoryDebugPanel from './panels/MemoryDebugPanel';
import MigrationPanel from './panels/MigrationPanel';
import NotificationsTabbedPanel from './panels/NotificationsTabbedPanel';
import PermissionsPanel from './panels/PermissionsPanel';
import PersonalityPanel from './panels/PersonalityPanel';
import PrivacyPanel from './panels/PrivacyPanel';
import ProfileEditorPage from './panels/ProfileEditorPage';
import ProfilesPanel from './panels/ProfilesPanel';
import RecoveryPhrasePanel from './panels/RecoveryPhrasePanel';
import SandboxSettingsPanel from './panels/SandboxSettingsPanel';
import ScreenAwarenessDebugPanel from './panels/ScreenAwarenessDebugPanel';
import SecurityPanel from './panels/SecurityPanel';
import TeamInvitesPanel from './panels/TeamInvitesPanel';
import TeamManagementPanel from './panels/TeamManagementPanel';
import TeamMembersPanel from './panels/TeamMembersPanel';
import TeamPanel from './panels/TeamPanel';
import ThemeStudioPanel from './panels/ThemeStudioPanel';
import ToolPolicyDiagnosticsPanel from './panels/ToolPolicyDiagnosticsPanel';
import ToolsPanel from './panels/ToolsPanel';
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
      {/* Usage & limits moved to the Connections page (cost / token savings /
          background loops as tabs). */}
      <Route path="usage" element={<Navigate to="/connections?tab=usage" replace />} />
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
      {/* Data Sync is a first-class surface on the Brain page now. */}
      <Route path="memory-sync" element={<Navigate to="/brain?tab=sync" replace />} />
      {/* Wallet balances moved to the Connections page (Integrations group). */}
      <Route path="wallet-balances" element={<Navigate to="/connections?tab=wallet" replace />} />
      <Route path="recovery-phrase" element={wrapSettingsPage(<RecoveryPhrasePanel />)} />

      {/* ── Connections ─────────────────────────────────────────── */}
      {/* The Integrations settings section was retired; the composio/OAuth grid
          lives on the Connections page. */}
      <Route path="integrations" element={<Navigate to="/connections" replace />} />
      {/* Screen Awareness / Desktop Agent / Desktop Companion moved to the
          Connections page (Desktop group). */}
      <Route
        path="screen-intelligence"
        element={<Navigate to="/connections?tab=screen-intelligence" replace />}
      />
      <Route
        path="desktop-agent"
        element={<Navigate to="/connections?tab=desktop-agent" replace />}
      />
      <Route path="tools" element={wrapSettingsPage(<ToolsPanel />)} />
      <Route path="companion" element={<Navigate to="/connections?tab=companion" replace />} />
      {/* Meeting settings moved to the Connections page (meetings tab). */}
      <Route path="meetings" element={<Navigate to="/connections?tab=meetings" replace />} />
      <Route path="autocomplete" element={wrapSettingsPage(<AutocompletePanel />)} />

      {/* ── System ──────────────────────────────────────────────── */}
      {/* Core connection — promotes cloud-mode remote-core config into a
          first-class setting with a live status indicator (GH-4396). */}
      <Route path="core" element={wrapSettingsPage(<CoreConnectionPanel />)} />
      <Route path="keyboard-shortcuts" element={wrapSettingsPage(<KeyboardShortcutsPanel />)} />
      <Route path="developer-options" element={wrapSettingsPage(<DeveloperOptionsPanel />)} />
      {/* Token savings merged into the Usage & limits surface on Connections. */}
      <Route path="token-usage" element={<Navigate to="/connections?tab=usage#tokens" replace />} />
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
      {/* Agent Chat debug tester moved to the Connections page. */}
      {/* Agent Chat is a chip on the Connections → LLM page. */}
      <Route
        path="agent-chat"
        element={<Navigate to="/connections?tab=llm#agent-chat" replace />}
      />
      <Route path="cron-jobs" element={wrapSettingsPage(<CronJobsPanel />)} />
      {/* Tasks now live on the Orchestration page's Kanban board. */}
      <Route path="tasks" element={<Navigate to="/orchestration?tab=tasks" replace />} />
      {/* Workflows is a first-level module now — /settings/automations bounces
          to /flows (the Workflows page). */}
      <Route path="automations" element={<Navigate to="/flows" replace />} />
      {/* Dev Workflow panel retired — superseded by Workflows (/flows). */}
      <Route path="dev-workflow" element={<Navigate to="/flows" replace />} />
      <Route path="skills-runner" element={wrapSettingsPage(<WorkflowRunnerPanel />)} />
      <Route
        path="screen-awareness-debug"
        element={wrapSettingsPage(<ScreenAwarenessDebugPanel />)}
      />
      <Route path="autocomplete-debug" element={wrapSettingsPage(<AutocompleteDebugPanel />)} />
      {/* Voice Debug page retired. */}
      <Route path="voice-debug" element={<SettingsRedirect to="/settings/developer-options" />} />
      {/* Local Model Debug is a chip on the Connections → LLM page. */}
      <Route
        path="local-model-debug"
        element={<Navigate to="/connections?tab=llm#local-model" replace />}
      />
      {/* Webhooks were retired from the UI — bounce old debug/trigger deep
          links to the Connections page. */}
      <Route path="webhooks-debug" element={<Navigate to="/connections" replace />} />
      <Route path="event-log" element={wrapSettingsPage(<EventLogPanel />)} />
      {/* Model Health page retired. */}
      <Route path="model-health" element={<SettingsRedirect to="/settings/developer-options" />} />
      {/* Memory inspection remains the configuration surface for the memory
          window, vault health, and connected-source controls. */}
      <Route path="memory-data" element={wrapSettingsPage(<MemoryDataPanel />)} />
      <Route path="memory-debug" element={wrapSettingsPage(<MemoryDebugPanel />)} />
      <Route path="analysis-views" element={<Navigate to="/brain" replace />} />
      <Route path="intelligence" element={<Navigate to="/brain" replace />} />
      {/* Composio trigger-triage config merged into the Connections Composio page. */}
      <Route
        path="composio-triggers"
        element={<Navigate to="/connections?tab=composio-key" replace />}
      />
      <Route path="permissions" element={wrapSettingsPage(<PermissionsPanel />)} />

      {/* ── Legacy slugs → redirects (deep-link compatibility) ──── */}
      {/* Old hub pages */}
      <Route path="ai" element={<Navigate to="/connections?tab=llm" replace />} />
      <Route path="agents-settings" element={<SettingsRedirect to="/settings/agents" />} />
      <Route
        path="features"
        element={<Navigate to="/connections?tab=screen-intelligence" replace />}
      />
      <Route path="crypto" element={<Navigate to="/connections?tab=wallet" replace />} />
      <Route path="notifications-hub" element={<SettingsRedirect to="/settings/notifications" />} />
      {/* Composio (API key + routing) moved to Connections → API keys. */}
      <Route path="composio" element={<Navigate to="/connections?tab=composio-key" replace />} />
      {/* Merged Usage & Limits surface (now on Connections) */}
      <Route
        path="heartbeat"
        element={<Navigate to="/connections?tab=usage#background" replace />}
      />
      <Route
        path="ledger-usage"
        element={<Navigate to="/connections?tab=usage#background" replace />}
      />
      <Route path="cost-dashboard" element={<Navigate to="/connections?tab=usage" replace />} />
      {/* Autonomy rate-limit lives inside Agent access now */}
      <Route path="autonomy" element={<SettingsRedirect to="/settings/agent-access" />} />
      {/* Merged Personality & Face page */}
      <Route path="mascot" element={<SettingsRedirect to="/settings/personality#face" />} />
      <Route path="persona" element={<SettingsRedirect to="/settings/personality" />} />
      {/* Retired Integrations settings section → Connections page */}
      <Route path="task-sources" element={<Navigate to="/connections" replace />} />
      <Route
        path="composio-routing"
        element={<Navigate to="/connections?tab=composio-key" replace />}
      />
      <Route path="webhooks-triggers" element={<Navigate to="/connections" replace />} />
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
