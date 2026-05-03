/**
 * Label applied to the welcome thread created when the user finishes the
 * desktop onboarding wizard. The thread is deleted once the welcome agent
 * calls `complete_onboarding(action: "complete")`. While it exists, the label
 * lets the UI hide all other threads during welcome lockdown and show a stable
 * "Onboarding" title.
 */
export const ONBOARDING_WELCOME_THREAD_LABEL = 'onboarding';
