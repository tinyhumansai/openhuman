import type { ReactNode } from 'react';
import { Navigate, Route } from 'react-router-dom';

import ForwardSearch from '../routing/ForwardSearch';
import SettingsIndexRedirect from './layout/SettingsIndexRedirect';
import AboutPanel from './panels/AboutPanel';
import AccountPanel from './panels/AccountPanel';
import AgentAccessPanel from './panels/AgentAccessPanel';
import AgentActivityPanel from './panels/AgentActivityPanel';
import AgentEditorPage from './panels/AgentEditorPage';
import AgentsPanel from './panels/AgentsPanel';
import AppearancePanel from './panels/AppearancePanel';
import ApprovalHistoryPanel from './panels/ApprovalHistoryPanel';
import BillingPanel from './panels/BillingPanel';
import CoreConnectionPanel from './panels/CoreConnectionPanel';
import DeveloperOptionsPanel from './panels/DeveloperOptionsPanel';
import DevicesPanel from './panels/DevicesPanel';
import EventLogPanel from './panels/EventLogPanel';
import FeedbackPanel from './panels/FeedbackPanel';
import McpServerPanel from './panels/McpServerPanel';
import MemoryDataPanel from './panels/MemoryDataPanel';
import MemoryDebugPanel from './panels/MemoryDebugPanel';
import MigrationPanel from './panels/MigrationPanel';
import NotificationsPanel from './panels/NotificationsPanel';
import PermissionsPanel from './panels/PermissionsPanel';
import PersonalityPanel from './panels/PersonalityPanel';
import PrivacyPanel from './panels/PrivacyPanel';
import ProfileEditorPage from './panels/ProfileEditorPage';
import ProfilesPanel from './panels/ProfilesPanel';
import RecoveryPhrasePanel from './panels/RecoveryPhrasePanel';
import SandboxSettingsPanel from './panels/SandboxSettingsPanel';
import SecurityPanel from './panels/SecurityPanel';
import ToolPolicyDiagnosticsPanel from './panels/ToolPolicyDiagnosticsPanel';
import ToolsPanel from './panels/ToolsPanel';
import WorkflowRunnerPanel from './panels/WorkflowRunnerPanel';

/**
 * Single vertical-scroll wrapper for a settings panel. The surrounding card
 * (bg / border / rounding) is provided by `SettingsLayout`'s content pane — so
 * panels sit directly on it. PanelScaffold-based panels are `h-full` and own their own
 * internal scroll; legacy panels that overflow scroll here. Either way there's
 * exactly one scrollbar.
 */
const WrappedSettingsPage = ({ children }: { children: ReactNode }) => {
  return <div className="h-full min-h-0 overflow-y-auto">{children}</div>;
};

const wrapSettingsPage = (element: ReactNode) => (
  <WrappedSettingsPage>{element}</WrappedSettingsPage>
);

/**
 * Redirect that stays *within* `/settings/*`. A thin alias for `<Navigate>`,
 * kept because it names the intent at ~10 call sites: these hops land on
 * another settings panel, while the external ones (`/brain`, `/connections`)
 * deliberately leave the settings tree.
 */
const SettingsRedirect = ({ to }: { to: string }) => <Navigate to={to} replace />;

/**
 * The full settings route table — index, every panel, and every legacy-slug
 * redirect. Returned as a fragment of `<Route>` elements (via a function call,
 * not a nested component) so it can be embedded directly inside a `<Routes>`:
 *
 *   `<Routes><Route element={<SettingsLayout/>}>{settingsRouteElements()}</Route></Routes>`
 *
 * Desktop and iOS both mount it that way through `pages/Settings.tsx`.
 *
 * Retired slugs are kept as redirects so deep links keep working.
 */
export function settingsRouteElements(): ReactNode {
  return (
    <>
      <Route index element={<SettingsIndexRedirect />} />

      {/* ── General ─────────────────────────────────────────────── */}
      <Route path="account" element={wrapSettingsPage(<AccountPanel />)} />
      {/* Teams were removed from the product. The slugs stay as redirects so
          existing deep links land on Account rather than reaching the settings
          index via the catch-all. */}
      <Route path="team" element={<SettingsRedirect to="/settings/account" />} />
      <Route path="team/*" element={<SettingsRedirect to="/settings/account" />} />
      <Route path="billing" element={wrapSettingsPage(<BillingPanel />)} />
      <Route path="privacy" element={wrapSettingsPage(<PrivacyPanel />)} />
      <Route path="security" element={wrapSettingsPage(<SecurityPanel />)} />
      <Route path="migration" element={wrapSettingsPage(<MigrationPanel />)} />
      <Route path="appearance" element={wrapSettingsPage(<AppearancePanel />)} />
      {/* Theme studio merged into Appearance — one page for one subject. */}
      <Route path="theme" element={<SettingsRedirect to="/settings/appearance" />} />
      <Route path="notifications" element={wrapSettingsPage(<NotificationsPanel />)} />
      {/* Real device-pairing panel (replaces the old "Coming Soon" stub). */}
      <Route path="devices" element={wrapSettingsPage(<DevicesPanel />)} />
      {/* Feedback was its own top-level route reached from a sidebar-header
          icon. That icon is the command-palette trigger now, which left the
          page with no way in, so the board lives here as a General panel. The
          old `/feedback` path redirects (see `AppRoutes`). */}
      <Route path="feedback" element={wrapSettingsPage(<FeedbackPanel />)} />

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
      <Route path="integrations" element={<ForwardSearch to="/connections" />} />
      <Route path="tools" element={wrapSettingsPage(<ToolsPanel />)} />

      {/* ── System ──────────────────────────────────────────────── */}
      {/* Core connection — promotes cloud-mode remote-core config into a
          first-class setting with a live status indicator (GH-4396). */}
      <Route path="core" element={wrapSettingsPage(<CoreConnectionPanel />)} />
      {/* Keyboard shortcuts is no longer a settings page — the in-app overlay
          (mod+/ or the sidebar's keyboard icon, `meta.keyboard-shortcuts`) is
          the one surface. The slug redirects so old links do not fall through
          to the settings index. */}
      <Route path="keyboard-shortcuts" element={<SettingsRedirect to="/settings/account" />} />
      <Route path="developer-options" element={wrapSettingsPage(<DeveloperOptionsPanel />)} />
      {/* Token savings merged into the Usage & limits surface on Connections. */}
      <Route path="token-usage" element={<Navigate to="/connections?tab=usage#tokens" replace />} />
      <Route path="about" element={wrapSettingsPage(<AboutPanel />)} />

      {/* ── Developer & Diagnostics leaf panels ─────────────────── */}
      <Route
        path="tool-policy-diagnostics"
        element={wrapSettingsPage(<ToolPolicyDiagnosticsPanel />)}
      />
      <Route path="mcp-server" element={wrapSettingsPage(<McpServerPanel />)} />
      {/* Search engine settings moved to the Connections page. */}
      <Route path="search" element={<Navigate to="/connections?tab=search" replace />} />
      {/* Agent Chat debug tester retired — the panel is deleted. The slug is
          kept as a redirect so an old deep link lands on the LLM page rather
          than the settings index via the catch-all. */}
      <Route path="agent-chat" element={<Navigate to="/connections?tab=llm" replace />} />
      {/* Schedules live on the Workflows page now (`/flows?view=schedules`). */}
      <Route path="cron-jobs" element={<Navigate to="/flows?view=schedules" replace />} />
      {/* Tasks are represented by goals on the Brain page. */}
      <Route path="tasks" element={<Navigate to="/brain?tab=goals" replace />} />
      {/* Workflows is a first-level module now — /settings/automations bounces
          to /flows (the Workflows page). */}
      <Route path="automations" element={<Navigate to="/flows" replace />} />
      {/* Dev Workflow panel retired — superseded by Workflows (/flows). */}
      <Route path="dev-workflow" element={<Navigate to="/flows" replace />} />
      <Route path="skills-runner" element={wrapSettingsPage(<WorkflowRunnerPanel />)} />
      {/* Voice Debug page retired. */}
      <Route path="voice-debug" element={<SettingsRedirect to="/settings/developer-options" />} />
      {/* Local Model Debug retired — the panel is deleted. Redirect kept for
          the same reason as agent-chat above. */}
      <Route path="local-model-debug" element={<Navigate to="/connections?tab=llm" replace />} />
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
      {/* The routing tab was removed; land on the notifications page itself. */}
      <Route
        path="notification-routing"
        element={<SettingsRedirect to="/settings/notifications" />}
      />
      {/* Fallback */}
      <Route path="*" element={<SettingsRedirect to="/settings" />} />
    </>
  );
}
