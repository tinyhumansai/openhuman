import { Navigate, Route, Routes } from 'react-router-dom';

import DefaultRedirect from './components/DefaultRedirect';
import ProtectedRoute from './components/ProtectedRoute';
import PublicRoute from './components/PublicRoute';
import Agents from './pages/Agents';
import Conversations from './pages/Conversations';
import Home from './pages/Home';
import Intelligence from './pages/Intelligence';
import Invites from './pages/Invites';
import Skills from './pages/Skills';
import Login from './pages/Login';
import Mnemonic from './pages/Mnemonic';
import Onboarding from './pages/onboarding/Onboarding';
import Settings from './pages/Settings';
import Welcome from './pages/Welcome';
import { selectHasEncryptionKey, selectIsOnboarded } from './store/authSelectors';
import { useAppSelector } from './store/hooks';

const OnboardingRoute = () => {
  const isOnboarded = useAppSelector(selectIsOnboarded);
  const hasEncryptionKey = useAppSelector(selectHasEncryptionKey);
  if (isOnboarded && !hasEncryptionKey) return <Navigate to="/mnemonic" replace />;
  if (isOnboarded) return <Navigate to="/home" replace />;
  return <Onboarding />;
};

const MnemonicRoute = () => {
  const hasEncryptionKey = useAppSelector(selectHasEncryptionKey);
  if (hasEncryptionKey) return <Navigate to="/home" replace />;
  return <Mnemonic />;
};

/**
 * Home route wrapper: shows Home by default.
 * Only redirects to onboarding when user profile is loaded and onboarding is not done.
 */
const HomeRoute = () => {
  const user = useAppSelector(state => state.user.user);
  const isOnboarded = useAppSelector(selectIsOnboarded);
  const hasEncryptionKey = useAppSelector(selectHasEncryptionKey);

  // While user profile is still loading, show Home (avoid flash to onboarding)
  if (!user) return <Home />;

  // User loaded but onboarding not done → redirect to onboarding
  if (!isOnboarded) return <Navigate to="/onboarding" replace />;

  // Onboarded but no encryption key → redirect to mnemonic page
  if (!hasEncryptionKey) return <Navigate to="/mnemonic" replace />;

  return <Home />;
};

const AppRoutes = () => {
  return (
    <>
      <Routes>
        {/* Public routes - redirect to /home or /onboarding if logged in */}
        <Route
          path="/"
          element={
            <PublicRoute>
              <Welcome />
            </PublicRoute>
          }
        />
        <Route
          path="/login"
          element={
            <PublicRoute>
              <Login />
            </PublicRoute>
          }
        />

        {/* Protected routes */}
        <Route
          path="/onboarding"
          element={
            <ProtectedRoute requireAuth={true} requireOnboarded={false}>
              <OnboardingRoute />
            </ProtectedRoute>
          }
        />
        <Route
          path="/mnemonic"
          element={
            <ProtectedRoute requireAuth={true}>
              <MnemonicRoute />
            </ProtectedRoute>
          }
        />
        <Route
          path="/home"
          element={
            <ProtectedRoute requireAuth={true}>
              <HomeRoute />
            </ProtectedRoute>
          }
        />

        {/* Intelligence */}
        <Route
          path="/intelligence"
          element={
            <ProtectedRoute requireAuth={true}>
              <Intelligence />
            </ProtectedRoute>
          }
        />

        {/* Skills */}
        <Route
          path="/skills"
          element={
            <ProtectedRoute requireAuth={true}>
              <Skills />
            </ProtectedRoute>
          }
        />

        {/* Conversations */}
        <Route
          path="/conversations"
          element={
            <ProtectedRoute requireAuth={true}>
              <Conversations />
            </ProtectedRoute>
          }
        />
        <Route
          path="/conversations/:threadId"
          element={
            <ProtectedRoute requireAuth={true}>
              <Conversations />
            </ProtectedRoute>
          }
        />

        {/* Invites */}
        <Route
          path="/invites"
          element={
            <ProtectedRoute requireAuth={true}>
              <Invites />
            </ProtectedRoute>
          }
        />

        {/* Agents */}
        <Route
          path="/agents"
          element={
            <ProtectedRoute requireAuth={true}>
              <Agents />
            </ProtectedRoute>
          }
        />

        {/* Settings - rendered as page content */}
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
    </>
  );
};

export default AppRoutes;
