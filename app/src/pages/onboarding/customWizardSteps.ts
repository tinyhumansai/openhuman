import type { CustomStepKey } from './OnboardingContext';

/** Ordered list of custom-wizard steps. Index drives the step counter UI and
 *  the back/continue navigation. */
export const CUSTOM_WIZARD_STEPS: CustomStepKey[] = [
  'inference',
  'voice',
  'oauth',
  'search',
  'embeddings',
  'activity',
  'vault',
  // 'memory',
];

export const CUSTOM_WIZARD_ROUTES: Record<CustomStepKey, string> = {
  inference: '/onboarding/custom/inference',
  voice: '/onboarding/custom/voice',
  oauth: '/onboarding/custom/oauth',
  search: '/onboarding/custom/search',
  embeddings: '/onboarding/custom/embeddings',
  activity: '/onboarding/custom/activity',
  memory: '/onboarding/custom/memory',
  vault: '/onboarding/custom/vault',
};
