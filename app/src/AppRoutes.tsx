import { Navigate, Route, Routes } from 'react-router-dom';

import AppRoutesIOS from './AppRoutesIOS';
import DefaultRedirect from './components/DefaultRedirect';
import ProtectedRoute from './components/ProtectedRoute';
import PublicRoute from './components/PublicRoute';
import HumanPage from './features/human/HumanPage';
import { getIsMobile } from './lib/platform';
import Accounts from './pages/Accounts';
import Channels from './pages/Channels';
import Home from './pages/Home';
import Intelligence from './pages/Intelligence';
import Invites from './pages/Invites';
import Notifications from './pages/Notifications';
import Onboarding from './pages/onboarding/Onboarding';
import Rewards from './pages/Rewards';
import Routines from './pages/Routines';
import Settings from './pages/Settings';
import SkillNew from './pages/SkillNew';
import Skills from './pages/Skills';
import SkillsRun from './pages/SkillsRun';
import WebCallbackPage from './pages/WebCallbackPage';
import Welcome from './pages/Welcome';

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

      <Route
        path="/human"
        element={
          <ProtectedRoute requireAuth={true}>
            <HumanPage />
          </ProtectedRoute>
        }
      />

      <Route
        path="/intelligence"
        element={
          <ProtectedRoute requireAuth={true}>
            <Intelligence />
          </ProtectedRoute>
        }
      />

      {/* Skills lives at /skills with its 4 sub-tabs (Composio / Channels /
          MCP Servers / Runners). The scheduled-skills dashboard concept
          composes INSIDE the Runners sub-tab, not as a separate top-level
          page — the bottom-bar "Connections" entry has always pointed at
          /skills to surface Composio integrations + MCP, and that muscle
          memory is restored here.
          `/skills/new` is the create-a-skill authoring page.
          Order matters: keep `/skills/new` before `/skills` so it wins the
          prefix match. */}
      <Route
        path="/skills/new"
        element={
          <ProtectedRoute requireAuth={true}>
            <SkillNew />
          </ProtectedRoute>
        }
      />

      <Route
        path="/skills/run"
        element={
          <ProtectedRoute requireAuth={true}>
            <SkillsRun />
          </ProtectedRoute>
        }
      />

      <Route
        path="/skills"
        element={
          <ProtectedRoute requireAuth={true}>
            <Skills />
          </ProtectedRoute>
        }
      />

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

      <Route
        path="/channels"
        element={
          <ProtectedRoute requireAuth={true}>
            <Channels />
          </ProtectedRoute>
        }
      />

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

      <Route
        path="/routines"
        element={
          <ProtectedRoute requireAuth={true}>
            <Routines />
          </ProtectedRoute>
        }
      />

      <Route
        path="/rewards"
        element={
          <ProtectedRoute requireAuth={true}>
            <Rewards />
          </ProtectedRoute>
        }
      />

      <Route path="/webhooks" element={<Navigate to="/settings/webhooks-triggers" replace />} />

      <Route
        path="/settings/*"
        element={
          <ProtectedRoute requireAuth={true}>
            <Settings />
          </ProtectedRoute>
        }
      />

      {/* Default redirect based on auth status */}
      <Route path="*" element={<DefaultRedirect />} />
    </Routes>
  );
};

export default AppRoutes;
