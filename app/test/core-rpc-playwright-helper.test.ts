import type { Page } from '@playwright/test';
import { describe, expect, it } from 'vitest';

import { seedBrowserCoreMode } from './playwright/helpers/core-rpc';

describe('Playwright core RPC helper', () => {
  it('primes walkthrough completion before the app renders', async () => {
    localStorage.clear();

    const page = {
      async addInitScript(
        script: (args: { rpcUrl: string; token: string }) => void,
        args: { rpcUrl: string; token: string }
      ) {
        script(args);
      },
    } as unknown as Page;

    await seedBrowserCoreMode(page);

    expect(localStorage.getItem('openhuman_core_mode')).toBe('cloud');
    expect(localStorage.getItem('openhuman:walkthrough_completed')).toBe('true');
    expect(localStorage.getItem('openhuman:walkthrough_pending')).toBeNull();
  });
});
