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
    id: 'model-downloads',
    title: 'Model downloads & updates',
    body: 'Local model weights are pulled from the model hub when you install or update them. After that, inference runs on your machine.',
  },
  {
    id: 'cloud-providers',
    title: 'Cloud AI providers — only when you pick one',
    body: 'If you route a task to OpenAI, Anthropic, a search API, or any webhook target, that message goes to them. Your choice, per task.',
  },
  {
    id: 'skill-integrations',
    title: 'Skill integrations you connect',
    body: "Skills like Gmail, Slack, or Notion talk to those services on your behalf — you authorized them. After each sync, a summary of that skill's state is sent to our memory service so the assistant can recall it across sessions.",
  },
  {
    id: 'sentry',
    title: 'Crash reports (opt-in)',
    body: 'Anonymous crash reports help us fix bugs. Toggle anytime in Settings → Privacy & Security.',
  },
  {
    id: 'auth-and-routing',
    title: 'Account, billing, and chat routing',
    body: 'Sign-in and billing go through our auth service. Chat messages are routed through our backend to the model you picked — local or cloud. Your files on disk stay on disk.',
  },
];

export const WHAT_LEAVES_HEADLINE = 'Local by default. Cloud when you ask.';
export const WHAT_LEAVES_SUBHEAD =
  "We won't claim nothing ever leaves your computer — that would be a lie. Here's exactly what does, and when.";
