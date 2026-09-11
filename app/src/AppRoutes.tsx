import { type Location, Navigate, Route, Routes } from 'react-router-dom';

import AppRoutesIOS from './AppRoutesIOS';
import DefaultRedirect from './components/DefaultRedirect';
import ProtectedRoute from './components/ProtectedRoute';
import PublicRoute from './components/PublicRoute';
import ForwardSearch from './components/routing/ForwardSearch';
import HumanPage from './features/human/HumanPage';
import { getIsMobile } from './lib/platform';
import Accounts from './pages/Accounts';
import Activity from './pages/Activity';
import Brain from './pages/Brain';
import AgentInsightsPreview from './pages/dev/AgentInsightsPreview';
import AssistantUiDemoPage from './pages/dev/assistant-ui-demo';
import UiGallery from './pages/dev/UiGallery';
import FlowCanvasPage, { FlowCanvasDraftPage } from './pages/FlowCanvasPage';
import FlowsPage from './pages/FlowsPage';
import Invites from './pages/Invites';
import Notifications from './pages/Notifications';
import Onboarding from './pages/onboarding/Onboarding';
import { PttOverlayPage } from './pages/PttOverlayPage';
import Rewards from './pages/Rewards';
import Settings from './pages/Settings';
import Skills from './pages/Skills';
import WebCallbackPage from './pages/WebCallbackPage';
import Welcome from './pages/Welcome';
import WorkflowsRun from './pages/WorkflowsRun';

interface AppRoutesProps {
  /**
   * Optional location override. Nothing passes one today — the router uses the
   * ambient location. It existed for the desktop Settings modal, which rendered
   * the page *behind* it from a stashed background location; Settings is a
   * routed page now. Kept because `<Routes location=…>` is the standard escape
   * hatch for any future overlay-over-a-page surface.
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

      {/* Human — the dedicated full-bleed mascot stage. The chat surface carries
          the same mascot docked on its composer; both read one set of mascot
          preferences from `mascotSlice`, so they cannot drift apart. */}
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

      {/* Back-compat: /activity and /intelligence → settings notifications page. */}
      <Route path="/activity" element={<Navigate to="/settings/notifications" replace />} />
      <Route path="/intelligence" element={<Navigate to="/settings/notifications" replace />} />

      {/* Connections page lives at /connections (Phase 2 rename from /skills).
          The old /skills path is kept as a back-compat redirect so bookmarks
          and deep links continue to work.  ForwardSearch copies the current
          ?tab= (and any other query params) to the destination so existing
          deep links still land on the right sub-tab. */}
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
      <Route path="/skills" element={<ForwardSearch to="/connections" />} />

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

      {/* Preserve links to the retired standalone accounts view. */}
      <Route path="/accounts" element={<Navigate to="/chat" replace />} />

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

      {/* Feedback is a settings panel now (`/settings/feedback`). Kept as a
          redirect rather than deleted: it was a real top-level route, so it is
          in users' history and in the walkthrough's deep links. */}
      <Route path="/feedback" element={<Navigate to="/settings/feedback" replace />} />

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
      <Route path="/webhooks" element={<ForwardSearch to="/settings/integrations" />} />

      {/* Settings is a routed page like every other surface: the shared route
          table renders inside `SettingsLayout`, which projects the settings nav
          into the app sidebar's dynamic region. It was a modal overlay (the
          backgroundLocation pattern) until this route replaced it. iOS keeps
          its own /settings/* route in AppRoutesIOS.tsx. */}
      <Route
        path="/settings/*"
        element={
          <ProtectedRoute requireAuth={true}>
            <Settings />
          </ProtectedRoute>
        }
      />

      <Route path="/ptt-overlay" element={<PttOverlayPage />} />

      {/* Dev-only visual preview of the Agentic task insights surface. */}
      <Route path="/dev/agent-insights" element={<AgentInsightsPreview />} />

      {/* Dev-only gallery of every shared UI primitive, in the active theme. */}
      <Route path="/dev/ui" element={<UiGallery />} />

      {/* Dev-only: the upstream assistant-ui `base` demo on a mock runtime. */}
      <Route path="/dev/assistant-ui" element={<AssistantUiDemoPage />} />

      {/* Default redirect based on auth status */}
      <Route path="*" element={<DefaultRedirect />} />
    </Routes>
  );
};

export default AppRoutes;
