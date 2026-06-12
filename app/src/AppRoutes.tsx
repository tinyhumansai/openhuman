import { Navigate, Route, Routes } from 'react-router-dom';

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
import Home from './pages/Home';
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
      <Route
        path="/home"
        element={
          <ProtectedRoute requireAuth={true}>
            <Home />
          </ProtectedRoute>
        }
      />

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

      {/* Primary Activity surface — replaces /intelligence (Phase 3). */}
      <Route
        path="/activity"
        element={
          <ProtectedRoute requireAuth={true}>
            <Activity />
          </ProtectedRoute>
        }
      />

      {/* Back-compat: /intelligence → /activity (preserves ?tab= deep links).
          Deep links such as ?tab=memory or ?tab=agents still resolve but fall
          back to the tasks tab in prod (dev-only tabs are gated inside Activity). */}
      <Route path="/intelligence" element={<Navigate to="/activity" replace />} />

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
        path="/chat"
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
      <Route path="/routines" element={<Navigate to="/activity?tab=automations" replace />} />

      <Route
        path="/rewards"
        element={
          <ProtectedRoute requireAuth={true}>
            <Rewards />
          </ProtectedRoute>
        }
      />

      {/* Workflows moved onto the Activity page (Automations tab). Keep the
          old /workflows path working as a deep link into that tab. */}
      <Route path="/workflows" element={<Navigate to="/activity?tab=automations" replace />} />

      <Route path="/webhooks" element={<Navigate to="/settings/webhooks-triggers" replace />} />

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

      {/* Default redirect based on auth status */}
      <Route path="*" element={<DefaultRedirect />} />
    </Routes>
  );
};

export default AppRoutes;
