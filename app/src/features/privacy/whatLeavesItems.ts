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
  // {
  //   id: 'model-downloads',
  //   title: 'Model downloads & updates',
  //   body: 'Local model weights are pulled from the model hub when you install or update them. After that, inference runs on your machine.',
  // },
  {
    id: 'cloud-providers',
    title: 'Cloud AI Inference',
    body: 'Most conversations are routed to the cloud for inference. This is the only way to get access to the most powerful AI models. Low-level tasks like summarizing chats are done using a local AI model.',
  },
  {
    id: 'skill-integrations',
    title: '3rd Party Integrations',
    body: '3rd Party integrations like Gmail, Slack, or Notion talk to those services on your behalf only with your explicit permission',
  },
  {
    id: 'sentry',
    title: 'Crash Reports & Usage Data (opt-out)',
    body: 'Anonymous crash reports help us fix bugs. Usage data helps us improve the product. Toggle anytime in Settings → Privacy & Security.',
  },
];

export const WHAT_LEAVES_HEADLINE = 'Local by default. Cloud when you ask.';
export const WHAT_LEAVES_SUBHEAD = "For full transparency, here's exactly what does, and when.";
