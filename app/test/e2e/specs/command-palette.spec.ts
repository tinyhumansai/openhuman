import { waitForApp } from '../helpers/app-helpers';
import { waitForWebView } from '../helpers/element-helpers';

// Dispatch a keydown on window (capture-phase hotkey listener lives there).
// `browser.keys()` is unreliable on tauri-driver, so we synthesize the event
// directly — this matches the manager's actual listener surface.
async function dispatchKey(
  key: string,
  opts: { meta?: boolean; ctrl?: boolean; shift?: boolean } = {}
): Promise<void> {
  await browser.execute(
    (k: string, meta: boolean, ctrl: boolean, shift: boolean) => {
      const ev = new KeyboardEvent('keydown', {
        key: k,
        metaKey: meta,
        ctrlKey: ctrl,
        shiftKey: shift,
        bubbles: true,
        cancelable: true,
      });
      window.dispatchEvent(ev);
    },
    key,
    !!opts.meta,
    !!opts.ctrl,
    !!opts.shift
  );
}

describe('Command palette', () => {
  before(async () => {
    await waitForApp();
    await waitForWebView();
  });

  it('opens via mod+K, runs an action, closes and navigates', async () => {
    await dispatchKey('k', { meta: true });

    const input = await browser.$('input[role="combobox"]');
    await input.waitForExist({ timeout: 5000 });

    await input.setValue('settings');
    await browser.keys('Enter');

    await browser.waitUntil(
      async () => {
        const hash = (await browser.execute('return window.location.hash')) as string;
        return typeof hash === 'string' && hash.includes('/settings');
      },
      { timeout: 5000, timeoutMsg: 'hash did not change to /settings' }
    );

    await browser.waitUntil(async () => !(await input.isExisting()), {
      timeout: 5000,
      timeoutMsg: 'palette did not close after Enter',
    });
  });

  it('palette lists the 5 seed nav actions, Esc closes', async () => {
    await dispatchKey('k', { meta: true });
    const input = await browser.$('input[role="combobox"]');
    await input.waitForExist({ timeout: 5000 });

    const seedLabels = [
      'Go Home',
      'Go to Chat',
      'Go to Intelligence',
      'Go to Skills',
      'Open Settings',
    ];
    for (const label of seedLabels) {
      const el = await browser.$(`*=${label}`);
      await el.waitForExist({ timeout: 2000, timeoutMsg: `seed action "${label}" missing` });
    }

    await dispatchKey('Escape');
    await browser.waitUntil(async () => !(await input.isExisting()), {
      timeout: 5000,
      timeoutMsg: 'palette did not close on Escape',
    });
  });

  it('regression probe: pre-existing keydown listeners still attached', async () => {
    // No dev-only handle is exposed by DictationHotkeyManager (Tauri OS-level
    // shortcut, not a DOM listener), so we probe window-level listener health
    // by asserting a fresh dispatch still reaches the command manager —
    // i.e. no prior test left the manager torn down / stack corrupted.
    await dispatchKey('k', { meta: true });
    const input = await browser.$('input[role="combobox"]');
    await input.waitForExist({ timeout: 5000 });
    await dispatchKey('Escape');
    await browser.waitUntil(async () => !(await input.isExisting()), {
      timeout: 5000,
      timeoutMsg: 'palette did not close — hotkey stack may be corrupted',
    });
  });
});
