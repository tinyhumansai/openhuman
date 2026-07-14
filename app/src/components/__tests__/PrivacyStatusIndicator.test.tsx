import { screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { PrivacyDisclosure } from '../../store/privacySlice';
import { renderWithProviders } from '../../test/test-utils';
import PrivacyStatusIndicator from '../PrivacyStatusIndicator';

function disclosure(over?: Partial<PrivacyDisclosure>): PrivacyDisclosure {
  return {
    id: 'd1',
    providerSlug: 'openai',
    service: 'OpenAI',
    isExternal: true,
    reason: 'inference',
    dataKinds: ['prompt'],
    riskLevel: 'unknown',
    riskCategories: [],
    receivedAt: 0,
    ...over,
  };
}

describe('PrivacyStatusIndicator (#4437 / S3)', () => {
  it('renders nothing until the privacy mode is hydrated', () => {
    const { container } = renderWithProviders(<PrivacyStatusIndicator />, {
      preloadedState: { privacy: { privacyMode: null, disclosuresByThread: {} } },
    });
    expect(container.firstChild).toBeNull();
  });

  it('shows the mode + on-device state when no external disclosure exists', () => {
    renderWithProviders(<PrivacyStatusIndicator />, {
      preloadedState: {
        privacy: { privacyMode: 'standard', disclosuresByThread: {} },
        thread: { selectedThreadId: 'thread-1' },
      },
    });
    const pill = screen.getByRole('status');
    expect(pill).toHaveTextContent('Standard');
    expect(pill).toHaveTextContent('On-device');
    expect(pill).toHaveAttribute('title', 'Standard · On-device');
  });

  it('shows the external state when the active thread has an external disclosure', () => {
    renderWithProviders(<PrivacyStatusIndicator />, {
      preloadedState: {
        privacy: { privacyMode: 'standard', disclosuresByThread: { 'thread-1': [disclosure()] } },
        thread: { selectedThreadId: 'thread-1' },
      },
    });
    const pill = screen.getByRole('status');
    expect(pill).toHaveTextContent('Sending externally');
    expect(pill).toHaveAttribute('title', 'Standard · Sending externally');
  });

  it('always reads on-device in local-only mode, even with a stale external disclosure', () => {
    renderWithProviders(<PrivacyStatusIndicator />, {
      preloadedState: {
        privacy: { privacyMode: 'local_only', disclosuresByThread: { 'thread-1': [disclosure()] } },
        thread: { selectedThreadId: 'thread-1' },
      },
    });
    const pill = screen.getByRole('status');
    expect(pill).toHaveTextContent('Local-only');
    expect(pill).toHaveTextContent('On-device');
  });

  it('ignores an external disclosure that belongs to a different thread', () => {
    renderWithProviders(<PrivacyStatusIndicator />, {
      preloadedState: {
        privacy: {
          privacyMode: 'standard',
          disclosuresByThread: { 'other-thread': [disclosure()] },
        },
        thread: { selectedThreadId: 'thread-1' },
      },
    });
    expect(screen.getByRole('status')).toHaveTextContent('On-device');
  });
});
