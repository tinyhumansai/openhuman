import type { Step } from 'react-joyride';

/**
 * Step definitions for the post-onboarding product walkthrough.
 * Targets must match `data-walkthrough="..."` attributes in the DOM.
 * Rendered by AppWalkthrough via react-joyride.
 */
export const WALKTHROUGH_STEPS: Step[] = [
  {
    target: '[data-walkthrough="home-card"]',
    title: 'Welcome to OpenHuman',
    content:
      'This is your home base. See your connection status and jump into a conversation from here.',
    placement: 'bottom',
    // v3 uses skipBeacon instead of disableBeacon
    skipBeacon: true,
  },
  {
    target: '[data-walkthrough="home-cta"]',
    title: 'Start chatting',
    content:
      'Tap here to message your AI assistant. Ask anything or get help with your connected services.',
    placement: 'bottom',
  },
  {
    target: '[data-walkthrough="tab-chat"]',
    title: 'Chat',
    content:
      'Your conversations live here. The AI assistant can help with tasks across all your connected apps.',
    placement: 'top',
  },
  {
    target: '[data-walkthrough="tab-skills"]',
    title: 'Skills & Connections',
    content: 'Connect your apps and manage what your assistant can do.',
    placement: 'top',
  },
  {
    target: '[data-walkthrough="tab-automation"]',
    title: 'Automation',
    content: 'Set up recurring tasks, scheduled agents, and proactive alerts.',
    placement: 'top',
  },
  {
    target: '[data-walkthrough="tab-settings"]',
    title: "You're all set!",
    content: 'Explore at your own pace. You can always reach your assistant through the Chat tab.',
    placement: 'top',
  },
];
