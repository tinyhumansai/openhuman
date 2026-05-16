import { Navigate, Route, Routes } from 'react-router-dom';

import OnboardingLayout from './OnboardingLayout';
import CustomInferencePage from './pages/CustomInferencePage';
// Search + Memory steps are hidden from the flow for now (files kept on
// disk; uncomment alongside customWizardSteps.ts to re-enable):
// import CustomMemoryPage from './pages/CustomMemoryPage';
import CustomOAuthPage from './pages/CustomOAuthPage';
// import CustomSearchPage from './pages/CustomSearchPage';
import CustomVoicePage from './pages/CustomVoicePage';
import RuntimeChoicePage from './pages/RuntimeChoicePage';
import WelcomePage from './pages/WelcomePage';

/**
 * Routed onboarding flow.
 *
 *   welcome → runtime-choice
 *     ├── cloud  → /home
 *     └── custom → /custom/inference → voice → oauth → search → memory → /home
 *
 * Each custom step asks Default (let OpenHuman manage it) vs Configure
 * (let me pick). Default is a one-click pick; Configure renders inline
 * controls (or a deep-link callout to Settings, for domains not yet
 * embedded). Gmail/Composio (`/onboarding/skills`) and Composio-driven
 * context gathering (`/onboarding/context`) are gone from the default
 * flow; their step + page files remain on disk in case we want to revive
 * them later.
 */
const Onboarding = () => {
  return (
    <Routes>
      <Route element={<OnboardingLayout />}>
        <Route index element={<Navigate to="welcome" replace />} />
        <Route path="welcome" element={<WelcomePage />} />
        <Route path="runtime-choice" element={<RuntimeChoicePage />} />
        <Route path="custom/inference" element={<CustomInferencePage />} />
        <Route path="custom/voice" element={<CustomVoicePage />} />
        <Route path="custom/oauth" element={<CustomOAuthPage />} />
        {/* <Route path="custom/search" element={<CustomSearchPage />} /> */}
        {/* <Route path="custom/memory" element={<CustomMemoryPage />} /> */}
        <Route path="*" element={<Navigate to="welcome" replace />} />
      </Route>
    </Routes>
  );
};

export default Onboarding;
