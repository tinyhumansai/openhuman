import { expect, type Page } from '@playwright/test';

const CORE_RPC_URL = process.env.PW_CORE_RPC_URL || 'http://127.0.0.1:17788/rpc';
const CORE_RPC_TOKEN = process.env.PW_CORE_RPC_TOKEN || 'openhuman-playwright-token';
const AUTH_CALLBACK_HOME_TIMEOUT_MS = 30_000;

let nextRpcId = 1;

interface JsonRpcSuccess<T> {
  result: T;
}

interface JsonRpcFailure {
  error: { message?: string; code?: number; data?: unknown };
}

function buildBypassJwt(userId: string): string {
  const payload = Buffer.from(
    JSON.stringify({ sub: userId, userId, exp: Math.floor(Date.now() / 1000) + 3600 })
  ).toString('base64url');
  return `eyJhbGciOiJub25lIiwidHlwIjoiSldUIn0.${payload}.sig`;
}

export async function callCoreRpc<T>(
  method: string,
  params: Record<string, unknown> = {}
): Promise<T> {
  const response = await fetch(CORE_RPC_URL, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json', Authorization: `Bearer ${CORE_RPC_TOKEN}` },
    body: JSON.stringify({ jsonrpc: '2.0', id: nextRpcId++, method, params }),
  });

  if (!response.ok) {
    throw new Error(`RPC ${method} failed with HTTP ${response.status}`);
  }

  const payload = (await response.json()) as JsonRpcSuccess<T> & JsonRpcFailure;
  if (payload.error) {
    throw new Error(`RPC ${method} failed: ${payload.error.message || 'unknown error'}`);
  }
  return payload.result;
}

export async function resetCoreForWebUser(userId: string): Promise<void> {
  await callCoreRpc('openhuman.auth_clear_session', {});
  await callCoreRpc('openhuman.config_set_onboarding_completed', { value: true });
  await callCoreRpc('openhuman.auth_store_session', { token: buildBypassJwt(userId) });
}

export async function seedBrowserCoreMode(page: Page): Promise<void> {
  await page.addInitScript(
    ({ rpcUrl, token }) => {
      window.localStorage.setItem('openhuman_core_mode', 'cloud');
      window.localStorage.setItem('openhuman_core_rpc_url', rpcUrl);
      window.localStorage.setItem('openhuman_core_rpc_token', token);
      window.localStorage.setItem('openhuman:walkthrough_completed', 'true');
      window.localStorage.removeItem('openhuman:walkthrough_pending');
    },
    { rpcUrl: CORE_RPC_URL, token: CORE_RPC_TOKEN }
  );
}

async function applyBrowserCoreModeInPage(page: Page): Promise<void> {
  await page.evaluate(
    ({ rpcUrl, token }) => {
      window.localStorage.setItem('openhuman_core_mode', 'cloud');
      window.localStorage.setItem('openhuman_core_rpc_url', rpcUrl);
      window.localStorage.setItem('openhuman_core_rpc_token', token);
      window.localStorage.setItem('openhuman:walkthrough_completed', 'true');
      window.localStorage.removeItem('openhuman:walkthrough_pending');
    },
    { rpcUrl: CORE_RPC_URL, token: CORE_RPC_TOKEN }
  );
}

async function completeAuthCallback(page: Page, token: string): Promise<void> {
  await page.goto(`/#/callback/auth?token=${encodeURIComponent(token)}&key=auth`);
  try {
    // The app-side auth callback waits up to 15s for CoreStateProvider to
    // commit currentUser before navigating to /home; CI occasionally needs
    // more than Playwright's default 10s assertion window here.
    await expect
      .poll(async () => page.evaluate(() => window.location.hash), {
        timeout: AUTH_CALLBACK_HOME_TIMEOUT_MS,
      })
      .toMatch(/^#\/home/);
    return;
  } catch {
    const runtimePickerVisible = await page
      .getByText(/Select a Runtime|Connect to Your Runtime/)
      .count()
      .then(count => count > 0)
      .catch(() => false);
    if (!runtimePickerVisible) {
      throw new Error(
        'auth callback did not reach /home and no runtime picker fallback was available'
      );
    }
  }

  await applyBrowserCoreModeInPage(page);
  await page.goto(`/#/callback/auth?token=${encodeURIComponent(token)}&key=auth`);
  await expect
    .poll(async () => page.evaluate(() => window.location.hash), {
      timeout: AUTH_CALLBACK_HOME_TIMEOUT_MS,
    })
    .toMatch(/^#\/home/);
}

export async function resetCoreForWebGuest(): Promise<void> {
  await callCoreRpc('openhuman.auth_clear_session', {});
  await callCoreRpc('openhuman.config_set_onboarding_completed', { value: true });
}

export async function bootRuntimeReadyGuestPage(page: Page): Promise<void> {
  await resetCoreForWebGuest();
  await seedBrowserCoreMode(page);
  await page.goto('/#/');
  await page.waitForSelector('#root');
}

export async function signInViaCallbackToken(page: Page, token: string): Promise<void> {
  await completeAuthCallback(page, token);
  await waitForAuthenticatedSnapshot(page);
  await waitForAppReady(page);
}

export async function signInViaBypassUser(page: Page, userId: string): Promise<void> {
  await completeAuthCallback(page, buildBypassJwt(userId));
  await waitForAuthenticatedSnapshot(page);
  await waitForAppReady(page);
}

export async function bootAuthenticatedPage(
  page: Page,
  userId: string,
  hash: string = '/home'
): Promise<void> {
  await resetCoreForWebUser(userId);
  await seedBrowserCoreMode(page);
  await page.goto('/#/home');
  await waitForAuthenticatedSnapshot(page);
  await page.goto(`/#${hash}`);
  await waitForAppReady(page);
}

export async function waitForAppReady(page: Page): Promise<void> {
  await page.waitForSelector('#root');
  await expect
    .poll(async () => {
      const text = await page
        .locator('#root')
        .innerText()
        .catch(() => '');
      return text.trim().length;
    })
    .toBeGreaterThan(20);
  await expect
    .poll(async () =>
      page.evaluate(() => {
        const candidates = Array.from(document.querySelectorAll('h2, button, p, div, span'));
        return candidates.some(node => {
          const text = node.textContent?.trim() ?? '';
          if (!/Select a Runtime|Connect to Your Runtime/.test(text)) return false;
          const el = node as HTMLElement;
          const rect = el.getBoundingClientRect();
          return rect.width > 0 && rect.height > 0;
        });
      })
    )
    .toBe(false);
}

export async function dismissWalkthroughIfPresent(page: Page): Promise<void> {
  const skipButton = page.getByRole('button', { name: /Skip|Skip tour/i });
  const portal = page.locator('#react-joyride-portal');
  const deadline = Date.now() + 5_000;
  const markCompleted = async () => {
    await page.evaluate(() => {
      try {
        localStorage.setItem('openhuman:walkthrough_completed', 'true');
        localStorage.removeItem('openhuman:walkthrough_pending');
      } catch {}
    });
  };

  while (Date.now() < deadline) {
    if ((await portal.count()) === 0) return;
    if (
      (await skipButton.count()) > 0 &&
      (await skipButton
        .first()
        .isVisible()
        .catch(() => false))
    ) {
      await markCompleted();
      await skipButton
        .first()
        .click({ force: true, timeout: 1_000 })
        .catch(() => {});
      try {
        await expect
          .poll(
            async () => {
              const visible = await skipButton
                .first()
                .isVisible()
                .catch(() => false);
              return !visible;
            },
            { timeout: 5_000 }
          )
          .toBe(true);
        return;
      } catch {
        // Some routes keep the Joyride portal mounted even after the tour is
        // dismissed. Keep looping so we can re-check visibility and fall back
        // to the persisted completion flag below.
      }
    }
    await page.waitForTimeout(100);
  }

  await markCompleted();
  // Last-resort: a lingering #react-joyride-portal node will keep
  // intercepting clicks on the page even after we've persisted the
  // completion flag, AND React may re-mount one later (e.g. after a
  // hash-route navigation that runs the walkthrough effect again).
  // Strip every portal node now AND install a MutationObserver that
  // keeps stripping any future mount for the rest of the page
  // lifetime. The observer install is idempotent — re-runs of this
  // helper on the same page no-op.
  await page.evaluate(() => {
    document.querySelectorAll('#react-joyride-portal').forEach(node => node.remove());
    const win = window as unknown as { __openhumanJoyrideScrubInstalled?: boolean };
    if (win.__openhumanJoyrideScrubInstalled) return;
    win.__openhumanJoyrideScrubInstalled = true;
    const scrub = (root: ParentNode) => {
      root.querySelectorAll('#react-joyride-portal').forEach(node => node.remove());
    };
    const obs = new MutationObserver(mutations => {
      for (const m of mutations) {
        m.addedNodes.forEach(node => {
          if (!(node instanceof Element)) return;
          if (node.id === 'react-joyride-portal') {
            node.remove();
          } else {
            scrub(node);
          }
        });
      }
    });
    obs.observe(document.body, { childList: true, subtree: true });
  });
}

async function waitForAuthenticatedSnapshot(page: Page): Promise<void> {
  await expect
    .poll(
      async () =>
        page.evaluate(() => {
          const winAny = window as unknown as {
            __OPENHUMAN_CORE_STATE__?: () => {
              snapshot?: {
                sessionToken?: string | null;
                currentUser?: { _id?: string | null } | null;
              };
            };
          };
          const snapshot = winAny.__OPENHUMAN_CORE_STATE__?.()?.snapshot;
          return {
            hasToken: Boolean(snapshot?.sessionToken),
            hasUser: Boolean(snapshot?.currentUser?._id),
          };
        }),
      { timeout: 20_000 }
    )
    .toEqual({ hasToken: true, hasUser: true });
}
