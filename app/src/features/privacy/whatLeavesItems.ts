export interface PrivacyLeaveItem {
  id: string;
  title: string;
  body: string;
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
    title: 'Crash Reports & Usage Data (opt-out)',
    body: 'Anonymous crash reports (via Sentry) and anonymous usage analytics — page views and feature engagement (via Google Analytics) — help us fix bugs and improve the product. No personal data, messages, or credentials are ever included. Toggle anytime in Settings → Privacy & Security.',
  },
];

export const WHAT_LEAVES_HEADLINE = 'Local by default. Cloud when you ask.';
export const WHAT_LEAVES_SUBHEAD = "For full transparency, here's exactly what does, and when.";
