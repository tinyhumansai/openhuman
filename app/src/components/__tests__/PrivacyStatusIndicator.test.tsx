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
      preloadedState: {
        privacy: { privacyMode: null, disclosuresByThread: {}, activeExternalByThread: {} },
      },
    });
    expect(container.firstChild).toBeNull();
  });

  it('shows the mode + on-device state when no external transfer is active', () => {
    renderWithProviders(<PrivacyStatusIndicator />, {
      preloadedState: {
        privacy: { privacyMode: 'standard', disclosuresByThread: {}, activeExternalByThread: {} },
        thread: { selectedThreadId: 'thread-1' },
      },
    });
    const pill = screen.getByRole('status');
    expect(pill).toHaveTextContent('Standard');
    expect(pill).toHaveTextContent('On-device');
    expect(pill).toHaveAttribute('title', 'Standard · On-device');
  });

  it('shows the off-device state when the active thread has a live external transfer', () => {
    renderWithProviders(<PrivacyStatusIndicator />, {
      preloadedState: {
        privacy: {
          privacyMode: 'standard',
          disclosuresByThread: {},
          activeExternalByThread: { 'thread-1': true },
        },
        thread: { selectedThreadId: 'thread-1' },
      },
    });
    const pill = screen.getByRole('status');
    expect(pill).toHaveTextContent('Off-device');
    expect(pill).toHaveAttribute('title', 'Standard · Off-device');
  });

  it('always reads on-device in local-only mode, even with a live external flag', () => {
    renderWithProviders(<PrivacyStatusIndicator />, {
      preloadedState: {
        privacy: {
          privacyMode: 'local_only',
          disclosuresByThread: {},
          activeExternalByThread: { 'thread-1': true },
        },
        thread: { selectedThreadId: 'thread-1' },
      },
    });
    const pill = screen.getByRole('status');
    expect(pill).toHaveTextContent('Local-only');
    expect(pill).toHaveTextContent('On-device');
  });

  it('ignores a live external transfer that belongs to a different thread', () => {
    renderWithProviders(<PrivacyStatusIndicator />, {
      preloadedState: {
        privacy: {
          privacyMode: 'standard',
          disclosuresByThread: {},
          activeExternalByThread: { 'other-thread': true },
        },
        thread: { selectedThreadId: 'thread-1' },
      },
    });
    expect(screen.getByRole('status')).toHaveTextContent('On-device');
  });

  // Regression (#4437 finding 1a): the pill's off-device state is driven by the
  // live transfer flag, NOT the dismissible disclosure ledger — so it reads
  // off-device even when the ledger has been emptied (e.g. the card dismissed)
  // while the transfer is still active.
  it('reads off-device from the live flag even when the disclosure ledger is empty', () => {
    renderWithProviders(<PrivacyStatusIndicator />, {
      preloadedState: {
        privacy: {
          privacyMode: 'standard',
          disclosuresByThread: {},
          activeExternalByThread: { 'thread-1': true },
        },
        thread: { selectedThreadId: 'thread-1' },
      },
    });
    expect(screen.getByRole('status')).toHaveTextContent('Off-device');
  });

  // Regression (#4437 finding 1b): a stale, un-dismissed ledger entry from an
  // earlier turn must NOT keep the pill off-device once the turn boundary
  // cleared the live flag. The pill ignores the ledger entirely.
  it('stays on-device with a stale ledger entry once the live flag is cleared', () => {
    renderWithProviders(<PrivacyStatusIndicator />, {
      preloadedState: {
        privacy: {
          privacyMode: 'standard',
          disclosuresByThread: { 'thread-1': [disclosure()] },
          activeExternalByThread: {},
        },
        thread: { selectedThreadId: 'thread-1' },
      },
    });
    expect(screen.getByRole('status')).toHaveTextContent('On-device');
  });
});
