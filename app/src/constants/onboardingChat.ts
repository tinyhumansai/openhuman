// [#1123] Commented out — welcome-agent onboarding replaced by Joyride walkthrough
// /**
//  * Label applied to the welcome thread created when the user finishes the
//  * desktop onboarding wizard. The thread is deleted once the welcome agent
//  * calls `complete_onboarding(action: "complete")`. While it exists, the label
//  * lets the UI hide all other threads during welcome lockdown and show a stable
//  * "Onboarding" title.
//  */
// export const ONBOARDING_WELCOME_THREAD_LABEL = 'onboarding';

/** @deprecated [#1123] — kept for any remaining imports; use empty string as placeholder */
export const ONBOARDING_WELCOME_THREAD_LABEL = 'onboarding';

/**
 * Pre-seeded welcome message shown in the chat panel at the end of the guided
 * tour (#1217). Surfaced as the agent's first message so new users land on
 * /chat with something to respond to.
 */
export const TOUR_WELCOME_MESSAGE =
  "Hey! Welcome to OpenHuman 👋 You just finished setting up, and I'm here whenever you need me. Ask me anything, get summaries from your connected apps, or just say hi — what would you like to explore first?";
