import { fireEvent, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { PrivacyDisclosure } from '../../../store/privacySlice';
import { renderWithProviders } from '../../../test/test-utils';
import { ExternalTransferDisclosureCard } from '../ExternalTransferDisclosureCard';

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

function renderCard(d: PrivacyDisclosure) {
  return renderWithProviders(
    <ExternalTransferDisclosureCard threadId="thread-1" disclosure={d} />,
    {
      preloadedState: {
        privacy: { privacyMode: 'standard', disclosuresByThread: { 'thread-1': [d] } },
      },
    }
  );
}

describe('ExternalTransferDisclosureCard (#4437 / S3)', () => {
  it('renders the title and a friendly what/where/why sentence', () => {
    renderCard(disclosure());
    const card = screen.getByRole('status');
    expect(card).toHaveTextContent('Leaving your device');
    expect(card).toHaveTextContent(
      'This will send your message to OpenAI (openai) because the AI model needs to process it.'
    );
  });

  it('joins multiple data kinds with friendly labels (never raw enums)', () => {
    renderCard(disclosure({ dataKinds: ['prompt', 'metadata'] }));
    const card = screen.getByRole('status');
    expect(card).toHaveTextContent('your message, request metadata');
    expect(card).not.toHaveTextContent('tool_arguments');
  });

  it('falls back to a generic label when data kinds are empty', () => {
    renderCard(disclosure({ dataKinds: [] }));
    expect(screen.getByRole('status')).toHaveTextContent('This will send data to OpenAI (openai)');
  });

  it('maps each reason to friendly copy', () => {
    renderCard(disclosure({ reason: 'network_fetch' }));
    expect(screen.getByRole('status')).toHaveTextContent('because a web request needs it');
  });

  it('is disclosure-only — no approve/deny buttons', () => {
    renderCard(disclosure());
    expect(screen.queryByRole('button', { name: /approve/i })).toBeNull();
    expect(screen.queryByRole('button', { name: /deny/i })).toBeNull();
    expect(screen.getByRole('button', { name: 'Got it' })).toBeInTheDocument();
  });

  it('dismisses the disclosure from the store on "Got it"', () => {
    const { store } = renderCard(disclosure());
    fireEvent.click(screen.getByRole('button', { name: 'Got it' }));
    expect(store.getState().privacy.disclosuresByThread['thread-1']).toBeUndefined();
  });
});
