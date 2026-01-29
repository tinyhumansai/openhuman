import { Routes, Route, Navigate } from "react-router-dom";
import Welcome from "./pages/Welcome";
import Login from "./pages/Login";
import Onboarding from "./pages/onboarding/Onboarding";
import Home from "./pages/Home";
import PublicRoute from "./components/PublicRoute";
import ProtectedRoute from "./components/ProtectedRoute";
import DefaultRedirect from "./components/DefaultRedirect";
import SettingsModal from "./components/settings/SettingsModal";
import { useAppSelector } from "./store/hooks";
import { selectIsOnboarded } from "./store/authSelectors";

const OnboardingRoute = () => {
  const isOnboarded = useAppSelector(selectIsOnboarded);

  // If the user has already completed onboarding, skip this page and go home.
  // On first load, when onboarding status is unset/false, we allow showing onboarding.
  if (isOnboarded) return <Navigate to="/home" replace />;
  return <Onboarding />;
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
          path="/home"
          element={
            <ProtectedRoute
              requireAuth={true}
              requireOnboarded={true}
              redirectTo="/onboarding"
            >
              <Home />
            </ProtectedRoute>
          }
        />

        {/* Settings modal routes - protected */}
        <Route
          path="/settings/*"
          element={
            <ProtectedRoute
              requireAuth={true}
              requireOnboarded={true}
              redirectTo="/onboarding"
            >
              <Home />
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
