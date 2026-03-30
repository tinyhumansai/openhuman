import { useEffect, useState } from 'react';
import { Navigate, Route, Routes } from 'react-router-dom';

import DefaultRedirect from './components/DefaultRedirect';
import ProtectedRoute from './components/ProtectedRoute';
import PublicRoute from './components/PublicRoute';
import RouteLoadingScreen from './components/RouteLoadingScreen';
import Agents from './pages/Agents';
import Conversations from './pages/Conversations';
import Home from './pages/Home';
import Intelligence from './pages/Intelligence';
import Invites from './pages/Invites';
import Mnemonic from './pages/Mnemonic';
import Onboarding from './pages/onboarding/Onboarding';
import Settings from './pages/Settings';
import Skills from './pages/Skills';
import Welcome from './pages/Welcome';
import { selectHasEncryptionKey, selectIsOnboarded } from './store/authSelectors';
import { useAppSelector } from './store/hooks';
import { DEV_FORCE_ONBOARDING } from './utils/config';
import {
  DEFAULT_WORKSPACE_ONBOARDING_FLAG,
  openhumanWorkspaceOnboardingFlagExists,
} from './utils/tauriCommands';

interface OnboardingRouteProps {
  hasWorkspaceOnboardingFlag: boolean;
  isWorkspaceFlagLoading: boolean;
}

const OnboardingRoute = ({
  hasWorkspaceOnboardingFlag,
  isWorkspaceFlagLoading,
}: OnboardingRouteProps) => {
  const isOnboarded = useAppSelector(selectIsOnboarded);
  const hasEncryptionKey = useAppSelector(selectHasEncryptionKey);
  const shouldSkipOnboarding = DEV_FORCE_ONBOARDING
    ? false
    : isOnboarded || hasWorkspaceOnboardingFlag;

  if (isWorkspaceFlagLoading) return <RouteLoadingScreen label="Loading workspace..." />;
  if (shouldSkipOnboarding && !hasEncryptionKey) return <Navigate to="/mnemonic" replace />;
  if (shouldSkipOnboarding) return <Navigate to="/home" replace />;
  return <Onboarding />;
};

const MnemonicRoute = () => {
  const hasEncryptionKey = useAppSelector(selectHasEncryptionKey);
  if (hasEncryptionKey) return <Navigate to="/home" replace />;
  return <Mnemonic />;
};

interface HomeRouteProps {
  hasWorkspaceOnboardingFlag: boolean;
  isWorkspaceFlagLoading: boolean;
}

/**
 * Home route wrapper: shows Home by default.
 * Only redirects to onboarding when user profile is loaded and onboarding is not done.
 */
const HomeRoute = ({ hasWorkspaceOnboardingFlag, isWorkspaceFlagLoading }: HomeRouteProps) => {
  const user = useAppSelector(state => state.user.user);
  const isOnboarded = useAppSelector(selectIsOnboarded);
  const hasEncryptionKey = useAppSelector(selectHasEncryptionKey);

  const shouldSkipOnboarding = DEV_FORCE_ONBOARDING
    ? false
    : isOnboarded || hasWorkspaceOnboardingFlag;

  // While user profile is still loading, show Home (avoid flash to onboarding)
  if (!user) return <Home />;

  // Keep Home mounted while workspace onboarding bypass check resolves
  if (isWorkspaceFlagLoading) return <Home />;

  // User loaded but onboarding not done → redirect to onboarding
  if (!shouldSkipOnboarding) return <Navigate to="/onboarding" replace />;

  // Onboarded but no encryption key → redirect to mnemonic page
  if (!hasEncryptionKey) return <Navigate to="/mnemonic" replace />;

  return <Home />;
};

const AppRoutes = () => {
  const [hasWorkspaceOnboardingFlag, setHasWorkspaceOnboardingFlag] = useState(false);
  const [isWorkspaceFlagLoading, setIsWorkspaceFlagLoading] = useState(true);

  useEffect(() => {
    if (DEV_FORCE_ONBOARDING) {
      setHasWorkspaceOnboardingFlag(false);
      setIsWorkspaceFlagLoading(false);
      return;
    }
    let mounted = true;
    const loadWorkspaceFlag = async () => {
      try {
        const hasFlag = await openhumanWorkspaceOnboardingFlagExists(
          DEFAULT_WORKSPACE_ONBOARDING_FLAG
        );
        if (mounted) setHasWorkspaceOnboardingFlag(hasFlag);
      } catch (error) {
        console.warn('[routing] failed to read workspace onboarding flag:', error);
        if (mounted) setHasWorkspaceOnboardingFlag(false);
      } finally {
        if (mounted) setIsWorkspaceFlagLoading(false);
      }
    };
    void loadWorkspaceFlag();

    return () => {
      mounted = false;
    };
  }, []);

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

        {/* Protected routes */}
        <Route
          path="/onboarding"
          element={
            <ProtectedRoute requireAuth={true} requireOnboarded={false}>
              <OnboardingRoute
                hasWorkspaceOnboardingFlag={hasWorkspaceOnboardingFlag}
                isWorkspaceFlagLoading={isWorkspaceFlagLoading}
              />
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
            <ProtectedRoute requireAuth={true} requireOnboarded={true}>
              <HomeRoute
                hasWorkspaceOnboardingFlag={hasWorkspaceOnboardingFlag}
                isWorkspaceFlagLoading={isWorkspaceFlagLoading}
              />
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
              <Navigate to="/conversations" replace />
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
