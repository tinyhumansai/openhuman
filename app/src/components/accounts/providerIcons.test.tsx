import { render } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { type AccountProvider, PROVIDERS } from '../../types/accounts';
import { ProviderIcon } from './providerIcons';

// Every provider the picker can surface, plus the hidden/legacy ids that
// still need to render correctly for existing accounts.
const ALL_PROVIDERS: AccountProvider[] = [
  'whatsapp',
  'wechat',
  'telegram',
  'linkedin',
  'slack',
  'discord',
  'gmail',
  'outlook',
  'instagram',
  'twitter',
  'google-meet',
  'zoom',
  'browserscan',
];

describe('ProviderIcon', () => {
  it('renders an SVG for every known provider (incl. gmail/outlook/instagram/twitter)', () => {
    for (const provider of ALL_PROVIDERS) {
      const { container, unmount } = render(<ProviderIcon provider={provider} />);
      expect(container.querySelector('svg'), `icon for ${provider}`).not.toBeNull();
      unmount();
    }
  });

  it('returns null for an unknown provider (default arm)', () => {
    const { container } = render(<ProviderIcon provider={'totally-unknown' as AccountProvider} />);
    expect(container.querySelector('svg')).toBeNull();
  });
});

describe('PROVIDERS registry', () => {
  it('exposes the newly added mail + social providers in the picker', () => {
    const ids = PROVIDERS.map(p => p.id);
    for (const id of ['gmail', 'outlook', 'instagram', 'twitter'] as const) {
      expect(ids, `${id} present in picker`).toContain(id);
    }
  });

  it('gives every provider a non-empty label, description and https service URL', () => {
    for (const p of PROVIDERS) {
      expect(p.label.length, `${p.id} label`).toBeGreaterThan(0);
      expect(p.description.length, `${p.id} description`).toBeGreaterThan(0);
      expect(p.serviceUrl, `${p.id} serviceUrl`).toMatch(/^https:\/\//);
    }
  });
});
