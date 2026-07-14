import { type Location, Navigate, Route, Routes } from 'react-router-dom';

import AgentWorldShell from './agentworld/AgentWorldShell';
import AgentWorld from './agentworld/pages/AgentWorld';
import AppRoutesIOS from './AppRoutesIOS';
import DefaultRedirect from './components/DefaultRedirect';
import ProtectedRoute from './components/ProtectedRoute';
import PublicRoute from './components/PublicRoute';
import HumanPage from './features/human/HumanPage';
import { getIsMobile } from './lib/platform';
import Accounts from './pages/Accounts';
import Activity from './pages/Activity';
import Brain from './pages/Brain';
import AgentInsightsPreview from './pages/dev/AgentInsightsPreview';
import Feedback from './pages/Feedback';
import FlowCanvasPage, { FlowCanvasDraftPage } from './pages/FlowCanvasPage';
import FlowsPage from './pages/FlowsPage';
import Invites from './pages/Invites';
import Notifications from './pages/Notifications';
import Onboarding from './pages/onboarding/Onboarding';
import OrchestrationPage from './pages/OrchestrationPage';
import { PttOverlayPage } from './pages/PttOverlayPage';
import Rewards from './pages/Rewards';
import Skills from './pages/Skills';
import WebCallbackPage from './pages/WebCallbackPage';
import Welcome from './pages/Welcome';
import WorkflowsRun from './pages/WorkflowsRun';

interface AppRoutesProps {
  /**
   * Optional location override. The desktop shell passes the *background*
   * location here while the Settings modal is open, so the page behind the
   * modal stays rendered even though the URL is `/settings/*`. Omitted
   * everywhere else (router uses the ambient location).
   */
  location?: Location | string;
}

const AppRoutes = ({ location }: AppRoutesProps = {}) => {
  // Mobile target (iOS or Android): pair → Human/Chat/Settings only.
  // Desktop routes are not rendered.
  if (getIsMobile()) {
    return <AppRoutesIOS />;
  }

  return (
    <Routes location={location}>
      {/* Public routes - redirect to /home if logged in */}
      <Route
        path="/"
        element={
          <PublicRoute>
            <Welcome />
          </PublicRoute>
        }
      />

      <Route path="/auth" element={<WebCallbackPage callbackKind="auth" />} />
      <Route path="/callback/:kind" element={<WebCallbackPage />} />
      <Route path="/callback/:kind/:status" element={<WebCallbackPage />} />

      {/* Onboarding (full-page stepper, gated by onboarding_completed) */}
      <Route
        path="/onboarding/*"
        element={
          <ProtectedRoute requireAuth={true}>
            <Onboarding />
          </ProtectedRoute>
        }
      />

      {/* Protected routes */}
      {/* Home is merged into the unified chat surface — /home redirects to /chat
          (the chat's empty "new window" state is the former Home greeting). */}
      <Route path="/home" element={<Navigate to="/chat" replace />} />

      {/* Human — first-class destination again (restored after the IA Phase 6
          merge into Assistant). Renders the Human/mascot surface. iOS serves
          /human via AppRoutesIOS. */}
      <Route
        path="/human"
        element={
          <ProtectedRoute requireAuth={true}>
            <HumanPage />
          </ProtectedRoute>
        }
      />

      {/* Brain — the centerpiece memory knowledge-graph surface, reached from
          the raised center button in the bottom bar. Full-page, graph-only. */}
      <Route
        path="/brain"
        element={
          <ProtectedRoute requireAuth={true}>
            <Brain />
          </ProtectedRoute>
        }
      />

      {/* Workflows — the `flows::` domain's discoverable list hub (issue
          B5a) plus the read-only Workflow Canvas (issue B5b.1) at
          `/flows/:id`. Distinct from the legacy SKILL.md `/workflows/*`
          Skill routes below (create/run); the bare `/workflows` and
          `/routines` slugs now redirect here (to `/flows`) since Workflows is
          a first-level module. Not a tab-level route (unlike `/flows` itself,
          `/flows/:id` isn't reached from the BottomTabBar), so
          `navigation.spec.ts`'s ROUTES table needs no change. Full editing
          (B5b.2+) and the agent-proposal surface (B4) are separate, later
          work. */}
      <Route
        path="/flows"
        element={
          <ProtectedRoute requireAuth={true}>
            <FlowsPage />
          </ProtectedRoute>
        }
      />
      {/* Unsaved draft canvas (Phase 4e) — the chat WorkflowProposalCard's
          "Open in canvas" action lands here with the proposed graph in
          `location.state`. Declared BEFORE `/flows/:id` so it matches first;
          otherwise `:id` would capture "draft" and try to `flows_get('draft')`.
          Opening a draft never persists — the canvas's own Save is the gate. */}
      <Route
        path="/flows/draft"
        element={
          <ProtectedRoute requireAuth={true}>
            <FlowCanvasDraftPage />
          </ProtectedRoute>
        }
      />
      <Route
        path="/flows/:id"
        element={
          <ProtectedRoute requireAuth={true}>
            <FlowCanvasPage />
          </ProtectedRoute>
        }
      />

      {/* Orchestration — TinyPlace multi-agent coordination surface, promoted
          from a Brain sub-tab into a first-class sidebar destination (sits
          right after Workflows). */}
      <Route
        path="/orchestration"
        element={
          <ProtectedRoute requireAuth={true}>
            <OrchestrationPage />
          </ProtectedRoute>
        }
      />
      {/* Back-compat: the old Brain deep link → the promoted top-level tab. */}
      <Route
        path="/brain/tinyplace-orchestration"
        element={<Navigate to="/orchestration" replace />}
      />

      {/* Back-compat: /activity and /intelligence → settings notifications page. */}
      <Route path="/activity" element={<Navigate to="/settings/notifications" replace />} />
      <Route path="/intelligence" element={<Navigate to="/settings/notifications" replace />} />

      {/* Connections page lives at /connections (Phase 2 rename from /skills).
          The old /skills path is kept as a back-compat redirect so bookmarks
          and deep links continue to work.  `?tab=` query params are preserved
          by Navigate (replace) so existing deep links still land on the right
          sub-tab. */}
      {/* `/workflows/run` is the single-purpose Skill runner page — the live
          destination of the Run button in the Automations tab (WorkflowsTab). */}
      <Route
        path="/workflows/run"
        element={
          <ProtectedRoute requireAuth={true}>
            <WorkflowsRun />
          </ProtectedRoute>
        }
      />

      <Route
        path="/connections"
        element={
          <ProtectedRoute requireAuth={true}>
            <Skills />
          </ProtectedRoute>
        }
      />

      {/* Back-compat: /skills → /connections (preserves ?tab= deep links). */}
      <Route path="/skills" element={<Navigate to="/connections" replace />} />

      {/* Unified chat = agent + connected web apps. Replaces the old
          /conversations and /accounts routes. */}
      <Route
        path="/chat/:threadId?"
        element={
          <ProtectedRoute requireAuth={true}>
            <Accounts />
          </ProtectedRoute>
        }
      />

      {/* Back-compat: /channels was an orphaned standalone page; it now
          redirects to the unified Connections page on the Messaging tab. */}
      <Route path="/channels" element={<Navigate to="/connections?tab=messaging" replace />} />

      <Route
        path="/invites"
        element={
          <ProtectedRoute requireAuth={true}>
            <Invites />
          </ProtectedRoute>
        }
      />

      <Route
        path="/feedback"
        element={
          <ProtectedRoute requireAuth={true}>
            <Feedback />
          </ProtectedRoute>
        }
      />

      <Route
        path="/notifications"
        element={
          <ProtectedRoute requireAuth={true}>
            <Notifications />
          </ProtectedRoute>
        }
      />

      {/* Back-compat: /routines was an orphaned dead page. Workflows is now a
          first-level module — redirect surviving deep links to /flows. */}
      <Route path="/routines" element={<Navigate to="/flows" replace />} />

      <Route
        path="/rewards"
        element={
          <ProtectedRoute requireAuth={true}>
            <Rewards />
          </ProtectedRoute>
        }
      />

      {/* Installed SKILL.md workflows remain a separate runtime surface from
          visual Flows. Keep the legacy top-level hub reachable. */}
      <Route
        path="/workflows"
        element={
          <ProtectedRoute requireAuth={true}>
            <Activity />
          </ProtectedRoute>
        }
      />

      {/* Webhooks retired from the UI — land on the Integrations settings. */}
      <Route path="/webhooks" element={<Navigate to="/settings/integrations" replace />} />

      {/* Desktop Settings renders as a modal overlay mounted by AppShellDesktop
          (App.tsx) using the backgroundLocation pattern — it is no longer an
          inline route here. iOS keeps its own /settings/* route in
          AppRoutesIOS.tsx. */}

      <Route path="/ptt-overlay" element={<PttOverlayPage />} />

      {/* Dev-only visual preview of the Agentic task insights surface. */}
      <Route path="/dev/agent-insights" element={<AgentInsightsPreview />} />

      {/* Agent World — tiny.place A2A social network integration.
          Nested routes (explore, directory, …) are handled inside AgentWorld. */}
      <Route
        path="/agent-world/*"
        element={
          <ProtectedRoute requireAuth={true}>
            <AgentWorldShell>
              <AgentWorld />
            </AgentWorldShell>
          </ProtectedRoute>
        }
      />

      {/* Default redirect based on auth status */}
      <Route path="*" element={<DefaultRedirect />} />
    </Routes>
  );
};

export default AppRoutes;
