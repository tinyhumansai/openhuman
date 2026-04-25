import { Route, Routes } from 'react-router-dom';

import DefaultRedirect from './components/DefaultRedirect';
import ProtectedRoute from './components/ProtectedRoute';
import PublicRoute from './components/PublicRoute';
import Accounts from './pages/Accounts';
import Channels from './pages/Channels';
import Home from './pages/Home';
import Intelligence from './pages/Intelligence';
import Invites from './pages/Invites';
import Notifications from './pages/Notifications';
import Onboarding from './pages/onboarding/Onboarding';
import Rewards from './pages/Rewards';
import Settings from './pages/Settings';
import Skills from './pages/Skills';
import Webhooks from './pages/Webhooks';
import Welcome from './pages/Welcome';

const AppRoutes = () => {
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
        path="/intelligence"
        element={
          <ProtectedRoute requireAuth={true}>
            <Intelligence />
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
        path="/rewards"
        element={
          <ProtectedRoute requireAuth={true}>
            <Rewards />
          </ProtectedRoute>
        }
      />

      <Route
        path="/webhooks"
        element={
          <ProtectedRoute requireAuth={true}>
            <Webhooks />
          </ProtectedRoute>
        }
      />

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
