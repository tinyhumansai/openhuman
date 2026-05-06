import { describe, expect, it } from 'vitest';

import { PROVIDERS } from './accounts';

describe('account provider registry', () => {
  it('includes Twitter as a supported embedded account service', () => {
    expect(PROVIDERS).toContainEqual(
      expect.objectContaining({
        id: 'twitter',
        label: 'Twitter',
        serviceUrl: 'https://x.com/home',
      })
    );
  });
});
