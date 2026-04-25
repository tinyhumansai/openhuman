import { Navigate, Route, Routes } from 'react-router-dom';

import OnboardingLayout from './OnboardingLayout';
import ChatProviderPage from './pages/ChatProviderPage';
import ContextPage from './pages/ContextPage';
import SkillsPage from './pages/SkillsPage';
import WelcomePage from './pages/WelcomePage';

/**
 * Routed onboarding flow. Each step is a real page under `/onboarding/*`
 * sharing chrome + draft state through {@link OnboardingLayout}. The flow
 * runs while `onboarding_completed` is false and ends by calling
 * `completeAndExit()` (persists the flag, navigates to /home).
 */
const Onboarding = () => {
  return (
    <Routes>
      <Route element={<OnboardingLayout />}>
        <Route index element={<Navigate to="welcome" replace />} />
        <Route path="welcome" element={<WelcomePage />} />
        <Route path="skills" element={<SkillsPage />} />
        <Route path="context" element={<ContextPage />} />
        <Route path="chat-provider" element={<ChatProviderPage />} />
        <Route path="*" element={<Navigate to="welcome" replace />} />
      </Route>
    </Routes>
  );
};

export default Onboarding;
