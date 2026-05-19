import { fireEvent, render, screen, within } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import type { GraphRelation } from '../../utils/tauriCommands';
import { MemoryInsights } from './MemoryInsights';

function makeRel(overrides: Partial<GraphRelation> = {}): GraphRelation {
  return {
    namespace: 'work',
    subject: 'Alice',
    predicate: 'is',
    object: 'a developer',
    attrs: {},
    updatedAt: 1700000000,
    evidenceCount: 1,
    orderIndex: null,
    documentIds: [],
    chunkIds: [],
    ...overrides,
  };
}

describe('<MemoryInsights />', () => {
  it('renders a loading skeleton when loading', () => {
    const { container } = render(<MemoryInsights relations={[]} loading />);
    expect(container.querySelectorAll('.animate-pulse').length).toBeGreaterThanOrEqual(3);
  });

  it('renders the empty state when there are no relations', () => {
    render(<MemoryInsights relations={[]} />);
    // The empty branch falls back to i18n keys but the structure is two text nodes.
    const headings = screen.getAllByRole('heading', { level: 3 });
    expect(headings.length).toBeGreaterThanOrEqual(1);
    expect(headings[0]).toBeInTheDocument();
  });

  it('groups relations into the expected predicate categories', () => {
    const relations = [
      makeRel({ subject: 'Alice', predicate: 'is', object: 'a developer' }), // facts
      makeRel({ subject: 'Bob', predicate: 'likes', object: 'jazz' }), // preferences
      makeRel({ subject: 'Carol', predicate: 'knows', object: 'Dave' }), // relationships
      makeRel({ subject: 'Eve', predicate: 'skilled_in', object: 'Rust' }), // skills
      makeRel({ subject: 'Frank', predicate: 'thinks', object: 'TS is great' }), // opinions
      makeRel({ subject: 'Grace', predicate: 'walks', object: 'fast' }), // other
    ];
    render(<MemoryInsights relations={relations} />);

    // Subjects from every bucket should be visible because there is one item
    // each (≤ 3 items always shown when collapsed).
    expect(screen.getByText('Alice')).toBeInTheDocument();
    expect(screen.getByText('Bob')).toBeInTheDocument();
    expect(screen.getByText('Carol')).toBeInTheDocument();
    expect(screen.getByText('Eve')).toBeInTheDocument();
    expect(screen.getByText('Frank')).toBeInTheDocument();
    expect(screen.getByText('Grace')).toBeInTheDocument();
  });

  it('sorts items inside a group by evidenceCount descending', () => {
    const relations = [
      makeRel({ subject: 'Low', predicate: 'is', evidenceCount: 1 }),
      makeRel({ subject: 'High', predicate: 'is', evidenceCount: 9 }),
      makeRel({ subject: 'Mid', predicate: 'is', evidenceCount: 4 }),
    ];
    render(<MemoryInsights relations={relations} />);
    const items = [
      screen.getByText('High').compareDocumentPosition(screen.getByText('Mid')),
      screen.getByText('Mid').compareDocumentPosition(screen.getByText('Low')),
    ];
    // Both comparisons should report "Mid follows High" / "Low follows Mid".
    expect(items[0] & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
    expect(items[1] & Node.DOCUMENT_POSITION_FOLLOWING).toBeTruthy();
  });

  it('shows the "+N more" hint when a group has more than 3 items', () => {
    const relations = Array.from({ length: 5 }, (_, i) =>
      makeRel({ subject: `S${i}`, predicate: 'is', object: 'thing', evidenceCount: i + 1 })
    );
    render(<MemoryInsights relations={relations} />);
    // 5 items in `facts`, collapsed view shows 3 → "+2 more"
    expect(screen.getByText(/\+2/)).toBeInTheDocument();
  });

  it('expands a group when its header is clicked, revealing more items', () => {
    const relations = Array.from({ length: 8 }, (_, i) =>
      makeRel({ subject: `S${i}`, predicate: 'is', object: 'thing', evidenceCount: i + 1 })
    );
    render(<MemoryInsights relations={relations} />);

    // Click the (only) category header — i18n key "insights.knownFacts".
    fireEvent.click(screen.getByRole('button', { name: /Known Facts/i }));

    // After expansion the "+N more" hint disappears and all subjects become visible.
    expect(screen.queryByText(/^\+/)).not.toBeInTheDocument();
    for (let i = 0; i < 8; i++) {
      expect(screen.getByText(`S${i}`)).toBeInTheDocument();
    }
  });

  it('decorates entity-type info as inline badges when present', () => {
    const relations = [
      makeRel({
        subject: 'Alice',
        predicate: 'is',
        object: 'developer',
        attrs: { entity_types: { subject: 'person', object: 'role' } },
      }),
    ];
    render(<MemoryInsights relations={relations} />);
    // The badge renders as a child of the subject span; query inside it
    // rather than reaching up to the row container.
    const subj = screen.getByText('Alice');
    expect(within(subj).getByText(/person/i)).toBeInTheDocument();
  });
});
