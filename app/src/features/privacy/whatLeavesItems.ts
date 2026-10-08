interface PrivacyLeaveItem {
  id: string;
  title: string;
  body: string;
  titleKey?: string;
  bodyKey?: string;
}

/**
 * The honest list of things that can leave the user's laptop.
 * Copy source: repo README + handoff doc. Do not soften this list —
 * the point is to not lie about "100% local".
 */
export const WHAT_LEAVES_ITEMS: PrivacyLeaveItem[] = [
  {
    id: 'cloud-providers',
    title: 'Cloud AI Inference',
    body: 'Core assistant features run locally by default. Cloud inference is only used when a feature explicitly needs stronger hosted models or network-backed services.',
  },
  {
    id: 'skill-integrations',
    title: 'Third-party integrations',
    body: 'Third-party integrations like Gmail, Slack, or Notion talk to those services on your behalf only with your explicit permission.',
  },
  {
    id: 'sentry',
    title: 'Crash reports and product analytics (opt-out)',
    body: 'Sentry crash reports and Google Analytics page views and feature usage help us improve the app. Change this choice in Settings → Privacy & Security. Agent trace sharing is a separate choice described below.',
    titleKey: 'privacy.whatLeaves.analytics.title',
    bodyKey: 'privacy.whatLeaves.analytics.body',
  },
  {
    id: 'agent-traces',
    title: 'Agent run traces (opt-in)',
    body: 'With your consent, OpenHuman sends timing and token usage data to Langfuse through its backend. A separate content setting adds prompts, replies, system prompts, and tool inputs and results. Sharing and content capture both start turned off.',
    titleKey: 'privacy.whatLeaves.traces.title',
    bodyKey: 'privacy.whatLeaves.traces.body',
  },
];

export const WHAT_LEAVES_HEADLINE = 'Local by default. Cloud when you ask.';
export const WHAT_LEAVES_SUBHEAD = "For full transparency, here's exactly what does, and when.";
