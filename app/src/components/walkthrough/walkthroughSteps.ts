import type { Step } from 'react-joyride';
import type { NavigateFunction } from 'react-router-dom';

import { TOUR_WELCOME_MESSAGE } from '../../constants/onboardingChat';
import { store } from '../../store';
import { addMessageLocal, createNewThread, setSelectedThread } from '../../store/threadSlice';
import type { ThreadMessage } from '../../types/thread';

type Translator = (key: string, fallback?: string) => string;

/**
 * Polls via setTimeout until `[data-walkthrough="<selector>"]` appears in the
 * DOM, then resolves. Rejects after `timeout` ms (default 3000).
 *
 * Uses setTimeout (not rAF) so tests can advance time with fake timers.
 */
export function waitForTarget(selector: string, timeout = 3000): Promise<void> {
  const POLL_INTERVAL = 50;

  return new Promise<void>((resolve, reject) => {
    let elapsed = 0;

    function check() {
      if (document.querySelector(`[data-walkthrough="${selector}"]`)) {
        resolve();
        return;
      }
      elapsed += POLL_INTERVAL;
      if (elapsed >= timeout) {
        reject(
          new Error(`[walkthrough] waitForTarget timed out: [data-walkthrough="${selector}"]`)
        );
        return;
      }
      setTimeout(check, POLL_INTERVAL);
    }

    // Initial check — element may already be present.
    if (document.querySelector(`[data-walkthrough="${selector}"]`)) {
      resolve();
      return;
    }
    setTimeout(check, POLL_INTERVAL);
  });
}

/**
 * Factory that produces the post-onboarding walkthrough sequence.
 *
 * Steps that navigate to a different page receive a `before` async hook that
 * calls `navigate(path)` and then waits for the target element to appear in
 * the DOM via `waitForTarget`.
 *
 * All targets follow the `[data-walkthrough="<name>"]` convention — add the
 * attribute to the corresponding DOM element in the page/component.
 */
export function createWalkthroughSteps(
  navigate: NavigateFunction,
  t: Translator = (_key, fallback) => fallback ?? _key
): Step[] {
  return [
    // ── Step 1 — /chat empty state ────────────────────────────────────────
    {
      target: '[data-walkthrough="home-card"]',
      title: t('walkthrough.steps.startChat.title'),
      content: t('walkthrough.steps.startChat.content'),
      placement: 'bottom',
      skipBeacon: true,
    },

    // ── Step 2 — /chat empty state ────────────────────────────────────────
    {
      target: '[data-walkthrough="home-cta"]',
      title: t('walkthrough.steps.sayHello.title'),
      content: t('walkthrough.steps.sayHello.content'),
      placement: 'bottom',
      skipBeacon: true,
    },

    // ── Step 3 — /chat ────────────────────────────────────────────────────
    {
      target: '[data-walkthrough="chat-agent-panel"]',
      title: t('walkthrough.steps.meetAi.title'),
      content: t('walkthrough.steps.meetAi.content'),
      placement: 'bottom',
      skipBeacon: true,
      before: async () => {
        navigate('/chat');
        await waitForTarget('chat-agent-panel');
      },
    },

    // ── Step 4 — /connections (Apps tab) ──────────────────────────────────
    {
      target: '[data-walkthrough="skills-grid"]',
      title: t('walkthrough.steps.connectWorld.title'),
      content: t('walkthrough.steps.connectWorld.content'),
      placement: 'top',
      skipBeacon: true,
      before: async () => {
        navigate('/connections');
        await waitForTarget('skills-grid');
      },
    },

    // ── Step 5 — /connections (Messaging tab) ────────────────────────────
    {
      target: '[data-walkthrough="skills-channels"]',
      title: t('walkthrough.steps.messagingApps.title'),
      content: t('walkthrough.steps.messagingApps.content'),
      placement: 'bottom',
      skipBeacon: true,
      before: async () => {
        await waitForTarget('skills-channels');
      },
    },

    // ── Step 6 — /settings ────────────────────────────────────────────────
    {
      target: '[data-walkthrough="settings-menu"]',
      title: t('walkthrough.steps.settings.title'),
      content: t('walkthrough.steps.settings.content'),
      placement: 'top',
      skipBeacon: true,
      before: async () => {
        navigate('/settings');
        await waitForTarget('settings-menu');
      },
    },

    // ── Step 7 — primary nav: Chat ────────────────────────────────────────
    {
      target: '[data-walkthrough="tab-chat"]',
      title: t('walkthrough.steps.chatTab.title'),
      content: t('walkthrough.steps.chatTab.content'),
      placement: 'top',
      skipBeacon: true,
      before: async () => {
        navigate('/chat');
        await waitForTarget('tab-chat');
      },
    },

    // ── Step 8 — primary nav: Human ───────────────────────────────────────
    {
      target: '[data-walkthrough="tab-human"]',
      title: t('walkthrough.steps.humanTab.title'),
      content: t('walkthrough.steps.humanTab.content'),
      placement: 'top',
      skipBeacon: true,
    },

    // ── Step 9 — primary nav: Brain ───────────────────────────────────────
    {
      target: '[data-walkthrough="tab-brain"]',
      title: t('walkthrough.steps.brainTab.title'),
      content: t('walkthrough.steps.brainTab.content'),
      placement: 'top',
      skipBeacon: true,
    },

    // ── Step 10 — primary nav: Agent World ────────────────────────────────
    {
      target: '[data-walkthrough="tab-agent-world"]',
      title: t('walkthrough.steps.agentWorldTab.title'),
      content: t('walkthrough.steps.agentWorldTab.content'),
      placement: 'top',
      skipBeacon: true,
    },

    // ── Step 11 — primary nav: Connections ────────────────────────────────
    {
      target: '[data-walkthrough="tab-connections"]',
      title: t('walkthrough.steps.connectionsTab.title'),
      content: t('walkthrough.steps.connectionsTab.content'),
      placement: 'top',
      skipBeacon: true,
    },

    // ── Step 12 — primary nav: Feedback ───────────────────────────────────
    {
      target: '[data-walkthrough="tab-feedback"]',
      title: t('walkthrough.steps.feedbackTab.title'),
      content: t('walkthrough.steps.feedbackTab.content'),
      placement: 'top',
      skipBeacon: true,
    },

    // ── Step 13 — /chat (pre-seeded welcome message) ──────────────────────
    {
      target: '[data-walkthrough="chat-agent-panel"]',
      title: t('walkthrough.steps.allSet.title'),
      content: t('walkthrough.steps.allSet.content'),
      placement: 'bottom',
      skipBeacon: true,
      before: async () => {
        try {
          const thread = await store.dispatch(createNewThread()).unwrap();
          const welcomeMessage: ThreadMessage = {
            id: `msg_${crypto.randomUUID()}`,
            content: TOUR_WELCOME_MESSAGE,
            type: 'text',
            sender: 'agent',
            createdAt: new Date().toISOString(),
            extraMetadata: {},
          };
          await store
            .dispatch(addMessageLocal({ threadId: thread.id, message: welcomeMessage }))
            .unwrap();
          store.dispatch(setSelectedThread(thread.id));
          navigate('/chat');
        } catch (err) {
          console.debug('[walkthrough] step-9 before hook failed, falling back to /chat', err);
          navigate('/chat');
        }
        await waitForTarget('chat-agent-panel');
      },
    },
  ];
}
