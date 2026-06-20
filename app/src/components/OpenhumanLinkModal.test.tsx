import { describe, expect, it } from 'vitest';

import type { AccountStatus } from '../types/accounts';
import { statusDisplay } from './OpenhumanLinkModal';

describe('statusDisplay', () => {
  it('maps every account lifecycle status to a translation key and dot color', () => {
    const cases: Array<[AccountStatus, string, string]> = [
      ['open', 'app.openhumanLink.status.connected', 'bg-emerald-500'],
      ['loading', 'app.openhumanLink.status.loading', 'bg-amber-400'],
      ['pending', 'app.openhumanLink.status.needsSignIn', 'bg-amber-400'],
      ['timeout', 'app.openhumanLink.status.timedOut', 'bg-red-400'],
      ['error', 'app.openhumanLink.status.error', 'bg-red-400'],
      ['closed', 'app.openhumanLink.status.closed', 'bg-stone-300'],
    ];

    for (const [status, labelKey, dotClass] of cases) {
      expect(statusDisplay(status)).toEqual({ labelKey, dotClass });
    }
  });

  it('returns a key under the app.openhumanLink.status namespace for every status', () => {
    const statuses: AccountStatus[] = ['open', 'loading', 'pending', 'timeout', 'error', 'closed'];
    for (const status of statuses) {
      expect(statusDisplay(status).labelKey).toMatch(/^app\.openhumanLink\.status\./);
    }
  });
});
