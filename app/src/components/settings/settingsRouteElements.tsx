import type { ReactNode } from 'react';
import { Navigate, Route, useLocation } from 'react-router-dom';

import ForwardSearch from '../routing/ForwardSearch';
import SettingsIndexRedirect from './layout/SettingsIndexRedirect';
import AboutPanel from './panels/AboutPanel';
import AccountPanel from './panels/AccountPanel';
import AgentAccessPanel from './panels/AgentAccessPanel';
import AppearancePanel from './panels/AppearancePanel';
import ApprovalHistoryPanel from './panels/ApprovalHistoryPanel';
import CoreConnectionPanel from './panels/CoreConnectionPanel';
import DeveloperOptionsPanel from './panels/DeveloperOptionsPanel';
import EventLogPanel from './panels/EventLogPanel';
import FeedbackPanel from './panels/FeedbackPanel';
import MascotPanel from './panels/MascotPanel';
import MigrationPanel from './panels/MigrationPanel';
import PermissionsPanel from './panels/PermissionsPanel';
import PersonaPanel from './panels/PersonaPanel';
import PrivacyPanel from './panels/PrivacyPanel';
import RecoveryPhrasePanel from './panels/RecoveryPhrasePanel';
import SandboxSettingsPanel from './panels/SandboxSettingsPanel';
import SecurityPanel from './panels/SecurityPanel';
import ThemeStudioPanel from './panels/ThemeStudioPanel';
import ToolPolicyDiagnosticsPanel from './panels/ToolPolicyDiagnosticsPanel';

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
 * Personality and Face used to be two tabs of one page, with Face at
 * `/settings/personality#face`. They are separate pages now; keep that old
 * deep link landing on Face.
 */
const PersonalityRoute = () => {
  const location = useLocation();
  if (location.hash === '#face') return <SettingsRedirect to="/settings/face" />;
  return wrapSettingsPage(<PersonaPanel />);
};

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
      <Route path="billing" element={<SettingsRedirect to="/settings/account" />} />
      <Route path="privacy" element={wrapSettingsPage(<PrivacyPanel />)} />
      <Route path="security" element={wrapSettingsPage(<SecurityPanel />)} />
      <Route path="migration" element={wrapSettingsPage(<MigrationPanel />)} />
      <Route path="appearance" element={wrapSettingsPage(<AppearancePanel />)} />
      {/* Theme studio merged into Appearance — one page for one subject. */}
      <Route path="theme" element={wrapSettingsPage(<ThemeStudioPanel />)} />
      {/* The Notifications settings page was removed entirely; the slug
          redirects to Account so old deep links / bookmarks still land
          somewhere real rather than falling through to the settings index. */}
      <Route path="notifications" element={<SettingsRedirect to="/settings/account" />} />
      {/* Real device-pairing panel (replaces the old "Coming Soon" stub). */}
      <Route path="devices" element={<SettingsRedirect to="/settings/account" />} />
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
      <Route path="personality" element={<PersonalityRoute />} />
      <Route path="face" element={wrapSettingsPage(<MascotPanel />)} />
      <Route path="language" element={<SettingsRedirect to="/settings/account" />} />
      {/* The Agents list and editor were removed; old links land on Connections → Tools. */}
      <Route path="agents/*" element={<Navigate to="/connections?tab=agent-tools" replace />} />
      <Route path="agent-access" element={wrapSettingsPage(<AgentAccessPanel />)} />
      {/* The agent activity level (background-AI knob) was retired. The slug
          redirects so any old deep link lands on Connections → Tools rather than falling
          through to the settings index. */}
      <Route
        path="activity-level"
        element={<Navigate to="/connections?tab=agent-tools" replace />}
      />
      <Route path="sandbox-settings" element={wrapSettingsPage(<SandboxSettingsPanel />)} />
      <Route path="approval-history" element={wrapSettingsPage(<ApprovalHistoryPanel />)} />

      {/* ── Data ────────────────────────────────────────────────── */}
      {/* Data Sync is the Memory page's Documents chip now. */}
      <Route
        path="memory-sync"
        element={<Navigate to="/connections?tab=brain&brain=documents" replace />}
      />
      {/* Wallet balances moved to the Connections page (Integrations group). */}
      <Route path="wallet-balances" element={<Navigate to="/connections?tab=wallet" replace />} />
      <Route path="recovery-phrase" element={wrapSettingsPage(<RecoveryPhrasePanel />)} />

      {/* ── Connections ─────────────────────────────────────────── */}
      {/* The Integrations settings section was retired; the composio/OAuth grid
          lives on the Connections page. */}
      <Route path="integrations" element={<ForwardSearch to="/connections" />} />
      {/* Tools moved to Connections → Tools. */}
      <Route path="tools" element={<Navigate to="/connections?tab=agent-tools" replace />} />

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
      <Route path="mcp-server" element={<SettingsRedirect to="/connections?tab=mcp" />} />
      {/* Search engine settings moved to the Connections page. */}
      <Route path="search" element={<Navigate to="/connections?tab=search" replace />} />
      {/* Agent Chat debug tester retired — the panel is deleted. The slug is
          kept as a redirect so an old deep link lands on the LLM page rather
          than the settings index via the catch-all. */}
      <Route path="agent-chat" element={<Navigate to="/connections?tab=llm" replace />} />
      {/* Schedules live on the Workflows page now (`/flows?view=schedules`). */}
      <Route path="cron-jobs" element={<Navigate to="/flows?view=schedules" replace />} />
      {/* Tasks were goals on the v1 Brain page; asking memory is the closest v2 surface. */}
      <Route path="tasks" element={<Navigate to="/connections?tab=brain&brain=ask" replace />} />
      {/* Workflows is a first-level module now — /settings/automations bounces
          to /flows (the Workflows page). */}
      <Route path="automations" element={<Navigate to="/flows" replace />} />
      {/* Dev Workflow panel retired — superseded by Workflows (/flows). */}
      <Route path="dev-workflow" element={<Navigate to="/flows" replace />} />
      <Route path="skills-runner" element={<SettingsRedirect to="/connections?tab=skills" />} />
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
      {/* Memory v2: the engine picker, synced sources and the memory
          inspector all live on the Memory page (Connections → Memory). */}
      <Route
        path="memory-engine"
        element={<Navigate to="/connections?tab=brain&brain=engine" replace />}
      />
      <Route
        path="memory-data"
        element={<Navigate to="/connections?tab=brain&brain=documents" replace />}
      />
      <Route
        path="memory-debug"
        element={<Navigate to="/connections?tab=brain&brain=ask" replace />}
      />
      <Route path="analysis-views" element={<Navigate to="/connections?tab=brain" replace />} />
      <Route path="intelligence" element={<Navigate to="/connections?tab=brain" replace />} />
      {/* Composio trigger-triage config merged into the Connections Composio page. */}
      <Route
        path="composio-triggers"
        element={<Navigate to="/connections?tab=composio-key" replace />}
      />
      <Route path="permissions" element={wrapSettingsPage(<PermissionsPanel />)} />

      {/* ── Legacy slugs → redirects (deep-link compatibility) ──── */}
      {/* Old hub pages */}
      <Route path="ai" element={<Navigate to="/connections?tab=llm" replace />} />
      <Route
        path="agents-settings"
        element={<Navigate to="/connections?tab=agent-tools" replace />}
      />
      <Route path="crypto" element={<Navigate to="/connections?tab=wallet" replace />} />
      <Route path="notifications-hub" element={<SettingsRedirect to="/settings/account" />} />
      {/* Composio (API key + routing) moved to Connections → API keys. */}
      <Route path="composio" element={<Navigate to="/connections?tab=composio-key" replace />} />
      {/* Merged Usage & Limits surface (now on Connections) */}
      <Route
        path="ledger-usage"
        element={<Navigate to="/connections?tab=usage#background" replace />}
      />
      <Route path="cost-dashboard" element={<Navigate to="/connections?tab=usage" replace />} />
      {/* Autonomy rate-limit lives inside Agent access now */}
      <Route path="autonomy" element={<SettingsRedirect to="/settings/agent-access" />} />
      {/* Merged Personality & Face page */}
      <Route path="mascot" element={<SettingsRedirect to="/settings/face" />} />
      <Route path="persona" element={<SettingsRedirect to="/settings/personality" />} />
      {/* Retired Integrations settings section → Connections page */}
      <Route path="task-sources" element={<Navigate to="/connections" replace />} />
      <Route
        path="composio-routing"
        element={<Navigate to="/connections?tab=composio-key" replace />}
      />
      <Route path="webhooks-triggers" element={<Navigate to="/connections" replace />} />
      {/* Notification routing tab and the Notifications settings page itself
          were both removed; land on Account instead. */}
      <Route path="notification-routing" element={<SettingsRedirect to="/settings/account" />} />
      {/* Fallback */}
      <Route path="*" element={<SettingsRedirect to="/settings" />} />
    </>
  );
}
