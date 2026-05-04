import type { Step } from 'react-joyride';

/**
 * Step definitions for the post-onboarding product walkthrough.
 * Targets must match `data-walkthrough="..."` attributes in the DOM.
 *
 * Copy is conversational and warm — matching OpenHuman's "calm sophistication"
 * design language. Each step has an emoji accent for visual interest.
 */
export const WALKTHROUGH_STEPS: Step[] = [
  {
    target: '[data-walkthrough="home-card"]',
    title: 'Your command center',
    content:
      "Everything starts here — your connections, your conversations, your AI. Think of this as mission control. Let's take a quick look around.",
    placement: 'bottom',
    skipBeacon: true,
  },
  {
    target: '[data-walkthrough="home-cta"]',
    title: 'Say hello',
    content:
      "This is the fastest way to talk to your AI assistant. Try asking it to summarize your emails, draft a message, or just say hi — it's surprisingly good at small talk.",
    placement: 'bottom',
  },
  {
    target: '[data-walkthrough="tab-chat"]',
    title: 'Conversations that remember',
    content:
      'Every chat is saved and searchable. Your assistant remembers context across conversations, so you can pick up right where you left off.',
    placement: 'top',
  },
  {
    target: '[data-walkthrough="tab-skills"]',
    title: 'Supercharge your assistant',
    content:
      'Connect Gmail, Slack, WhatsApp, and more. The more you connect, the more your assistant can actually do — not just talk about doing.',
    placement: 'top',
  },
  {
    target: '[data-walkthrough="tab-automation"]',
    title: 'Set it and forget it',
    content:
      'Morning briefings, scheduled check-ins, proactive alerts. Your assistant can work for you even when you are not looking.',
    placement: 'top',
  },
  {
    target: '[data-walkthrough="tab-settings"]',
    title: "You're in control",
    content:
      "That's the quick tour! You can always find settings, billing, and preferences here. Now go explore — your assistant is ready when you are.",
    placement: 'top',
  },
];

