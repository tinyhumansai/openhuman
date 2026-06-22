import { Navigate, Route, Routes } from 'react-router-dom';

import AgentWorldShell from './agentworld/AgentWorldShell';
import AgentWorld from './agentworld/pages/AgentWorld';
import AppRoutesIOS from './AppRoutesIOS';
import DefaultRedirect from './components/DefaultRedirect';
import ProtectedRoute from './components/ProtectedRoute';
import PublicRoute from './components/PublicRoute';
import HumanPage from './features/human/HumanPage';
import { getIsMobile } from './lib/platform';
import Accounts from './pages/Accounts';
import Brain from './pages/Brain';
import AgentInsightsPreview from './pages/dev/AgentInsightsPreview';
import Feedback from './pages/Feedback';
import Invites from './pages/Invites';
import Notifications from './pages/Notifications';
import Onboarding from './pages/onboarding/Onboarding';
import { PttOverlayPage } from './pages/PttOverlayPage';
import Rewards from './pages/Rewards';
import Settings from './pages/Settings';
import Skills from './pages/Skills';
import WebCallbackPage from './pages/WebCallbackPage';
import Welcome from './pages/Welcome';
import WorkflowNew from './pages/WorkflowNew';
import WorkflowsRun from './pages/WorkflowsRun';

const AppRoutes = () => {
  // Mobile target (iOS or Android): pair → Human/Chat/Settings only.
  // Desktop routes are not rendered.
  if (getIsMobile()) {
    return <AppRoutesIOS />;
  }

  return (
    <Routes>
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

      {/* Back-compat: /activity and /intelligence → settings notifications page. */}
      <Route path="/activity" element={<Navigate to="/settings/notifications" replace />} />
      <Route path="/intelligence" element={<Navigate to="/settings/notifications" replace />} />

      {/* Connections page lives at /connections (Phase 2 rename from /skills).
          The old /skills path is kept as a back-compat redirect so bookmarks
          and deep links continue to work.  `?tab=` query params are preserved
          by Navigate (replace) so existing deep links still land on the right
          sub-tab.
          `/workflows/new` is the create-a-skill authoring page.
          Order matters: keep `/workflows/new` before `/connections` so it wins
          the prefix match. */}
      <Route
        path="/workflows/new"
        element={
          <ProtectedRoute requireAuth={true}>
            <WorkflowNew />
          </ProtectedRoute>
        }
      />

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

      {/* Back-compat: /routines was an orphaned dead page (superseded by the
          Cron Jobs settings panel).  Redirect to Activity → Automations so
          any surviving deep links land somewhere sensible. */}
      <Route path="/routines" element={<Navigate to="/settings/automations" replace />} />

      <Route
        path="/rewards"
        element={
          <ProtectedRoute requireAuth={true}>
            <Rewards />
          </ProtectedRoute>
        }
      />

      <Route path="/workflows" element={<Navigate to="/settings/automations" replace />} />

      <Route path="/webhooks" element={<Navigate to="/settings/integrations#webhooks" replace />} />

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
