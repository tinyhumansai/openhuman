import { waitForApp, waitForAppReady } from '../helpers/app-helpers';
import { waitForWebView } from '../helpers/element-helpers';
import { resetApp } from '../helpers/reset-app';
import { startMockServer, stopMockServer } from '../mock-server';

// Map option names to WebDriver key strings (W3C Actions API codes).
const WD_KEY: Record<string, string> = { meta: '\uE03D', ctrl: '\uE009', shift: '\uE008' };

// `mod` in the product means Cmd on macOS, Ctrl on Linux/Windows
// (see app/src/lib/commands/shortcut.ts:matchEvent). Mirror that here so
// the dispatched event has the modifier the matcher actually checks.
const MOD_KEY: { meta?: boolean; ctrl?: boolean } =
  process.platform === 'darwin' ? { meta: true } : { ctrl: true };

// Dispatch a key combination to the active page.
//
// Primary: WebDriver Actions API via CDP `Input.dispatchKeyEvent` — this
// injects a real key event into the Chromium renderer's input pipeline and
// reliably reaches `window.addEventListener('keydown', ..., { capture:true })`.
//
// Fallback: synthetic DOM event (kept for older driver compat).
async function dispatchKey(
  key: string,
  opts: { meta?: boolean; ctrl?: boolean; shift?: boolean } = {}
): Promise<void> {
  // Build the modifier sequence for the Actions API.
  const mods: string[] = [];
  if (opts.meta) mods.push(WD_KEY.meta);
  if (opts.ctrl) mods.push(WD_KEY.ctrl);
  if (opts.shift) mods.push(WD_KEY.shift);

  try {
    // Use the W3C Key Action source — CDP translates this to
    // Input.dispatchKeyEvent which fires a native-level keydown in the
    // renderer. This is more reliable than a synthetic DOM event because it
    // goes through Chromium's own input dispatch path.
    let action = browser.action('key');
    for (const mod of mods) action = action.down(mod);
    action = action.down(key);
    action = action.up(key);
    for (const mod of [...mods].reverse()) action = action.up(mod);
    await action.perform();
  } catch {
    // Fallback: synthetic DOM KeyboardEvent dispatched directly on window.
    // Reaches capture-phase listeners even when the Actions API is unavailable.
    await browser.execute(
      (k: string, meta: boolean, ctrl: boolean, shift: boolean) => {
        window.dispatchEvent(
          new KeyboardEvent('keydown', {
            key: k,
            metaKey: meta,
            ctrlKey: ctrl,
            shiftKey: shift,
            bubbles: true,
            cancelable: true,
          })
        );
      },
      key,
      !!opts.meta,
      !!opts.ctrl,
      !!opts.shift
    );
  }
}

// Close an overlay via Escape, escalating to a document-targeted synthetic
// event as a last resort. ModalShell's `useEscapeKey` binds to `document`, so a
// `window`-dispatched fallback would miss it — dispatch on `document` directly.
async function closeOverlayWithEscape(el: WebdriverIO.Element, timeoutMsg: string): Promise<void> {
  try {
    await browser.keys('Escape');
  } catch {
    await dispatchKey('Escape');
  }
  try {
    await browser.waitUntil(async () => !(await el.isExisting()), { timeout: 3000 });
  } catch {
    await browser.execute(() => {
      document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
    });
    await browser.waitUntil(async () => !(await el.isExisting()), { timeout: 3000, timeoutMsg });
  }
}

describe('Command palette', () => {
  before(async () => {
    // CommandProvider is mounted inside the auth-gated provider chain.
    // We must be logged in or mod+K will find no listener.
    await startMockServer();
    await waitForApp();
    await waitForWebView();
    await resetApp('e2e-command-palette');
    await waitForAppReady(10_000);
  });

  after(async () => {
    await stopMockServer();
  });

  it('opens via mod+K, runs an action, closes and navigates', async () => {
    // Retry mod+K up to 3 times — WebDriver Actions API can silently drop the
    // first dispatch when the focus context hasn't settled yet.
    let input: WebdriverIO.Element | undefined;
    for (let attempt = 0; attempt < 3; attempt++) {
      await dispatchKey('k', MOD_KEY);
      input = await browser.$('input[role="combobox"]');
      try {
        await input.waitForExist({ timeout: 3000 });
        break;
      } catch {
        if (attempt === 2) throw new Error('Command palette did not open after 3 mod+K attempts');
      }
    }

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
    for (let attempt = 0; attempt < 3; attempt++) {
      await dispatchKey('k', MOD_KEY);
      const probe = await browser.$('input[role="combobox"]');
      try {
        await probe.waitForExist({ timeout: 3000 });
        break;
      } catch {
        if (attempt === 2) throw new Error('Command palette did not open after 3 mod+K attempts');
      }
    }
    const input = await browser.$('input[role="combobox"]');
    // Wait for cmdk to render [cmdk-item] elements — typically 200-400ms.
    await browser.waitUntil(
      async () => {
        const count = await browser.execute(() => document.querySelectorAll('[cmdk-item]').length);
        return count >= 3;
      },
      { timeout: 5000, interval: 200, timeoutMsg: 'cmdk items did not render' }
    );

    // Labels mirror app/src/lib/commands/globalActions.ts. The IA rename
    // retired "Go to Intelligence"/"Go to Skills" in favour of
    // "Go to Knowledge & Memory"/"Go to Connections".
    const seedLabels = [
      'Go Home',
      'Go to Chat',
      'Go to Connections',
      'Go to Knowledge & Memory',
      'Open Settings',
    ];
    for (const label of seedLabels) {
      const found = await browser.execute((lbl: string) => {
        const items = document.querySelectorAll('[cmdk-item]');
        return Array.from(items).some(el => el.textContent?.includes(lbl));
      }, label);
      expect(found).toBe(true);
    }

    // Close the palette — try browser.keys first (real keyboard), then
    // dispatchKey fallback, then programmatic close.
    try {
      await browser.keys('Escape');
    } catch {
      await dispatchKey('Escape');
    }
    try {
      await browser.waitUntil(async () => !(await input.isExisting()), { timeout: 3000 });
    } catch {
      // Programmatic close as last resort.
      await browser.execute(() => {
        document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
      });
      await browser.waitUntil(async () => !(await input.isExisting()), {
        timeout: 3000,
        timeoutMsg: 'palette did not close on Escape',
      });
    }
  });

  it('regression probe: pre-existing keydown listeners still attached', async () => {
    // No dev-only handle is exposed by DictationHotkeyManager (Tauri OS-level
    // shortcut, not a DOM listener), so we probe window-level listener health
    // by asserting a fresh dispatch still reaches the command manager —
    // i.e. no prior test left the manager torn down / stack corrupted.
    for (let attempt = 0; attempt < 3; attempt++) {
      await dispatchKey('k', MOD_KEY);
      const probe = await browser.$('input[role="combobox"]');
      try {
        await probe.waitForExist({ timeout: 3000 });
        break;
      } catch {
        if (attempt === 2) throw new Error('Command palette did not open after 3 mod+K attempts');
      }
    }
    const input = await browser.$('input[role="combobox"]');
    try {
      await browser.keys('Escape');
    } catch {
      await dispatchKey('Escape');
    }
    try {
      await browser.waitUntil(async () => !(await input.isExisting()), { timeout: 3000 });
    } catch {
      await browser.execute(() => {
        document.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true }));
      });
      await browser.waitUntil(async () => !(await input.isExisting()), {
        timeout: 3000,
        timeoutMsg: 'palette did not close — hotkey stack may be corrupted',
      });
    }
  });

  it('opens the keyboard-shortcuts help via mod+/ and lists grouped shortcuts', async () => {
    // `mod+/` is allowed even while an input is focused, so it reliably opens
    // the help directory regardless of where focus currently sits.
    let list = await browser.$('[data-testid="keyboard-shortcuts-list"]');
    for (let attempt = 0; attempt < 3; attempt++) {
      await dispatchKey('/', MOD_KEY);
      list = await browser.$('[data-testid="keyboard-shortcuts-list"]');
      try {
        await list.waitForExist({ timeout: 3000 });
        break;
      } catch {
        if (attempt === 2) throw new Error('Shortcuts help did not open after 3 mod+/ attempts');
      }
    }

    // The directory renders live from the command registry — assert a couple of
    // the new global actions and a group heading are present.
    for (const label of ['New Chat', 'Toggle Sidebar', 'Navigation']) {
      const found = await browser.execute((lbl: string) => {
        const root = document.querySelector('[data-testid="keyboard-shortcuts-list"]');
        return !!root && (root.textContent ?? '').includes(lbl);
      }, label);
      expect(found).toBe(true);
    }

    // Esc closes the overlay (ModalShell's useEscapeKey).
    await closeOverlayWithEscape(list, 'shortcuts help did not close on Escape');
  });

  it('opens the keyboard-shortcuts help via the ? key', async () => {
    // `?` must NOT fire while a text field is focused (so users can still type
    // a literal "?"), so blur first to emulate pressing it from app chrome.
    await browser.execute(() => (document.activeElement as HTMLElement | null)?.blur?.());

    let list = await browser.$('[data-testid="keyboard-shortcuts-list"]');
    for (let attempt = 0; attempt < 3; attempt++) {
      await dispatchKey('?');
      list = await browser.$('[data-testid="keyboard-shortcuts-list"]');
      try {
        await list.waitForExist({ timeout: 3000 });
        break;
      } catch {
        if (attempt === 2) throw new Error('Shortcuts help did not open after 3 ? attempts');
      }
    }
    expect(await list.isExisting()).toBe(true);

    await closeOverlayWithEscape(list, 'shortcuts help did not close');
  });
});
