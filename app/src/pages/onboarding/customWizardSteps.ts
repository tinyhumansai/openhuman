import type { CustomStepKey } from './OnboardingContext';

/** Ordered list of custom-wizard steps. Index drives the step counter UI and
 *  the back/continue navigation. `search` and `memory` are commented out for
 *  now — their pages still exist and route in case we want to re-enable. */
export const CUSTOM_WIZARD_STEPS: CustomStepKey[] = [
  'inference',
  'voice',
  'oauth',
  // 'search',
  // 'memory',
];

export const CUSTOM_WIZARD_ROUTES: Record<CustomStepKey, string> = {
  inference: '/onboarding/custom/inference',
  voice: '/onboarding/custom/voice',
  oauth: '/onboarding/custom/oauth',
  search: '/onboarding/custom/search',
  memory: '/onboarding/custom/memory',
};

/** Deep-link target inside Settings for users who pick "Configure" and want
 *  to finish wiring this domain up after onboarding. */
export const CUSTOM_WIZARD_SETTINGS_ROUTES: Record<CustomStepKey, string> = {
  inference: '/settings/llm',
  voice: '/settings/voice',
  oauth: '/settings/connections',
  search: '/settings/tools',
  memory: '/settings/memory-data',
};
