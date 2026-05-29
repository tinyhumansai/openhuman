import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands/memory';
import BeliefLedger from './BeliefLedger';

const NOW = 1_700_000_000;
const DAY = 86_400;

function makeRel(overrides: Partial<GraphRelation> = {}): GraphRelation {
  return {
    namespace: 'work',
    subject: 'You',
    predicate: 'lives_in',
    object: 'Berlin',
    attrs: {},
    updatedAt: NOW,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
    ...overrides,
  };
}

const contested: GraphRelation[] = [
  makeRel({
    predicate: 'lives_in',
    object: 'Berlin',
    updatedAt: NOW - 120 * DAY,
    evidenceCount: 3,
  }),
  makeRel({ predicate: 'lives_in', object: 'Lisbon', updatedAt: NOW, evidenceCount: 2 }),
];

describe('<BeliefLedger />', () => {
  it('renders a loading skeleton when loading', () => {
    const { container } = render(<BeliefLedger relations={[]} loading nowSeconds={NOW} />);
    expect(screen.getByTestId('belief-ledger-loading')).toBeInTheDocument();
    expect(container.querySelectorAll('.animate-pulse').length).toBeGreaterThanOrEqual(3);
  });

  it('renders the empty state when there are no relations', () => {
    render(<BeliefLedger relations={[]} nowSeconds={NOW} />);
    expect(screen.getByText('No beliefs to show yet.')).toBeInTheDocument();
  });

  it('renders a single reinforced claim with 100% confidence and no conflict badge', () => {
    render(<BeliefLedger relations={[makeRel({ object: 'Berlin' })]} nowSeconds={NOW} />);
    const card = screen.getByRole('article', { name: 'You lives_in' });
    // 'Berlin' appears in both the header (current belief) and the timeline.
    expect(within(card).getAllByText('Berlin').length).toBeGreaterThanOrEqual(1);
    expect(within(card).getByText('100%')).toBeInTheDocument();
    expect(within(card).queryByText('Contested')).not.toBeInTheDocument();
  });

  it('flags a contradicted claim and shows the current best belief (newest value)', () => {
    render(<BeliefLedger relations={contested} nowSeconds={NOW} />);
    const card = screen.getByRole('article', { name: 'You lives_in' });
    expect(within(card).getByText('Contested')).toBeInTheDocument();
    // Current belief (header, font-medium) is Lisbon; both values appear in the timeline.
    expect(within(card).getAllByText('Lisbon').length).toBeGreaterThanOrEqual(1);
    expect(within(card).getByText('Berlin')).toBeInTheDocument();
    // Revision history present.
    expect(within(card).getByText('Revision history')).toBeInTheDocument();
  });

  it('fires onExplain only for contested claims and onCorrect for any claim', () => {
    const onExplain = vi.fn();
    const onCorrect = vi.fn();
    render(
      <BeliefLedger
        relations={contested}
        nowSeconds={NOW}
        onExplain={onExplain}
        onCorrect={onCorrect}
      />
    );
    fireEvent.click(screen.getByRole('button', { name: /Explain this contradiction/ }));
    expect(onExplain).toHaveBeenCalledTimes(1);
    expect(onExplain.mock.calls[0][0].current.normalizedObject).toBe('lisbon');
    fireEvent.click(screen.getByRole('button', { name: /Set the truth/ }));
    expect(onCorrect).toHaveBeenCalledTimes(1);
  });

  it('does not render an Explain button for a non-contested claim', () => {
    const onExplain = vi.fn();
    render(<BeliefLedger relations={[makeRel()]} nowSeconds={NOW} onExplain={onExplain} />);
    expect(
      screen.queryByRole('button', { name: /Explain this contradiction/ })
    ).not.toBeInTheDocument();
  });

  it('shows the "explaining…" state on the card awaiting an explanation', () => {
    // Build the claimKey the engine produces for this claim.
    render(
      <BeliefLedger
        relations={contested}
        nowSeconds={NOW}
        onExplain={() => {}}
        explainingKey={'you␟lives in'}
      />
    );
    expect(screen.getByRole('button', { name: /Explain this contradiction/ })).toHaveTextContent(
      'Explaining…'
    );
  });

  it('renders an inline explanation when provided for the claim', () => {
    render(
      <BeliefLedger
        relations={contested}
        nowSeconds={NOW}
        explanationByKey={{ 'you␟lives in': 'They relocated from Berlin to Lisbon.' }}
      />
    );
    expect(screen.getByText('Explanation')).toBeInTheDocument();
    expect(screen.getByText('They relocated from Berlin to Lisbon.')).toBeInTheDocument();
  });
});
