import type { Step } from 'react-joyride';
import type { NavigateFunction } from 'react-router-dom';

import { TOUR_WELCOME_MESSAGE } from '../../constants/onboardingChat';
import { store } from '../../store';
import { addMessageLocal, createNewThread, setSelectedThread } from '../../store/threadSlice';
import type { ThreadMessage } from '../../types/thread';

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
 * Factory that produces the 10-step walkthrough sequence.
 *
 * Steps that navigate to a different page receive a `before` async hook that
 * calls `navigate(path)` and then waits for the target element to appear in
 * the DOM via `waitForTarget`.
 *
 * All targets follow the `[data-walkthrough="<name>"]` convention — add the
 * attribute to the corresponding DOM element in the page/component.
 */
export function createWalkthroughSteps(navigate: NavigateFunction): Step[] {
  return [
    // ── Step 1 — /home ────────────────────────────────────────────────────
    {
      target: '[data-walkthrough="home-card"]',
      title: 'Your command center',
      content:
        "This is your home base — a quick snapshot of what's happening and what needs your attention.",
      placement: 'bottom',
      skipBeacon: true,
    },

    // ── Step 2 — /home ────────────────────────────────────────────────────
    {
      target: '[data-walkthrough="home-cta"]',
      title: 'Say hello',
      content: 'Tap here to start a conversation with your AI assistant anytime.',
      placement: 'bottom',
      skipBeacon: true,
    },

    // ── Step 3 — /chat ────────────────────────────────────────────────────
    {
      target: '[data-walkthrough="chat-agent-panel"]',
      title: 'Meet your AI',
      content:
        'This is where conversations happen. Ask questions, get summaries, or brainstorm. Everything stays searchable.',
      placement: 'bottom',
      skipBeacon: true,
      before: async () => {
        navigate('/chat');
        await waitForTarget('chat-agent-panel');
      },
    },

    // ── Step 4 — /skills ──────────────────────────────────────────────────
    {
      target: '[data-walkthrough="skills-grid"]',
      title: 'Connect your world',
      content:
        'Gmail, Slack, WhatsApp, and more — each connection gives your assistant superpowers.',
      placement: 'top',
      skipBeacon: true,
      before: async () => {
        navigate('/skills');
        await waitForTarget('skills-grid');
      },
    },

    // ── Step 5 — /skills (channels) ─────────────────────────────────────
    {
      target: '[data-walkthrough="skills-channels"]',
      title: 'Chat where you already are',
      content:
        'WhatsApp, Telegram, Slack, Discord — connect your messaging apps so your assistant can reach you anywhere.',
      placement: 'bottom',
      skipBeacon: true,
      before: async () => {
        await waitForTarget('skills-channels');
      },
    },

    // ── Step 6 — /intelligence ────────────────────────────────────────────
    {
      target: '[data-walkthrough="intelligence-header"]',
      title: "Your assistant's brain",
      content:
        'This is where your assistant learns and remembers. It gets smarter the more you use it.',
      placement: 'bottom',
      skipBeacon: true,
      before: async () => {
        navigate('/intelligence');
        await waitForTarget('intelligence-header');
      },
    },

    // ── Step 6 — /settings ────────────────────────────────────────────────
    {
      target: '[data-walkthrough="settings-menu"]',
      title: 'Make it yours',
      content:
        'Preferences, privacy, notifications — everything is here. You can restart this tour anytime from this page.',
      placement: 'top',
      skipBeacon: true,
      before: async () => {
        navigate('/settings');
        await waitForTarget('settings-menu');
      },
    },

    // ── Step 7 — /home ────────────────────────────────────────────────────
    {
      target: '[data-walkthrough="tab-chat"]',
      title: 'Quick access',
      content: 'These tabs are your shortcuts — always one tap away.',
      placement: 'top',
      skipBeacon: true,
      before: async () => {
        navigate('/home');
        await waitForTarget('tab-chat');
      },
    },

    // ── Step 8 — /home (already there) ───────────────────────────────────
    {
      target: '[data-walkthrough="tab-notifications"]',
      title: 'Stay in the loop',
      content: 'Alerts and automations live here — briefings, notifications, background activity.',
      placement: 'top',
      skipBeacon: true,
    },

    // ── Step 9 — /chat (pre-seeded welcome message) ───────────────────────
    {
      target: '[data-walkthrough="chat-agent-panel"]',
      title: "You're all set!",
      content:
        'Your assistant left you a welcome note — this is your space to chat, ask questions, or brainstorm. Have fun!',
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
