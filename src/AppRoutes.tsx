import { useEffect, useState } from 'react';
import { Navigate, Route, Routes } from 'react-router-dom';

import DefaultRedirect from './components/DefaultRedirect';
import ProtectedRoute from './components/ProtectedRoute';
import PublicRoute from './components/PublicRoute';
import SettingsModal from './components/settings/SettingsModal';
import Home from './pages/Home';
import Login from './pages/Login';
import Onboarding from './pages/onboarding/Onboarding';
import Welcome from './pages/Welcome';
import { selectIsOnboarded } from './store/authSelectors';
import { useAppSelector } from './store/hooks';
import { isTauri } from './utils/tauriCommands';

const OnboardingRoute = () => {
  const isOnboarded = useAppSelector(selectIsOnboarded);
  if (isOnboarded) return <Navigate to="/home" replace />;
  return <Onboarding />;
};

/**
 * Home route wrapper: shows Home by default.
 * Only redirects to onboarding when user profile is loaded and onboarding is not done.
 */
const HomeRoute = () => {
  const user = useAppSelector(state => state.user.user);
  const isOnboarded = useAppSelector(selectIsOnboarded);

  // While user profile is still loading, show Home (avoid flash to onboarding)
  if (!user) return <Home />;

  // User loaded but onboarding not done → redirect to onboarding
  if (!isOnboarded) return <Navigate to="/onboarding" replace />;

  return <Home />;
};

const AppRoutes = () => {
  const [isWeb, setIsWeb] = useState(false);

  useEffect(() => {
    // Check if we're running on web (not Tauri)
    setIsWeb(!isTauri());
  }, []);

  if (isWeb) return <Welcome isWeb={isWeb} />;

  return (
    <>
      <Routes>
        {/* Public routes - redirect to /home or /onboarding if logged in */}
        <Route
          path="/"
          element={
            <PublicRoute>
              <Welcome isWeb={isWeb} />
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
          path="/home"
          element={
            <ProtectedRoute requireAuth={true}>
              <HomeRoute />
            </ProtectedRoute>
          }
        />

        {/* Settings modal routes - protected */}
        <Route
          path="/settings/*"
          element={
            <ProtectedRoute requireAuth={true}>
              <HomeRoute />
            </ProtectedRoute>
          }
        />

        {/* Default redirect based on auth status */}
        <Route path="*" element={<DefaultRedirect />} />
      </Routes>

      {/* Settings Modal - renders over existing content when on settings routes */}
      <SettingsModal />
    </>
  );
};

export default AppRoutes;
