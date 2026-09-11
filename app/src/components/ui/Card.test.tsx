import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import Card from './Card';

/** The body is the element that directly wraps the children. */
const bodyOf = (childTestId = 'body') => screen.getByTestId(childTestId).parentElement!;

describe('Card', () => {
  it('renders a bordered surface around its children', () => {
    render(
      <Card data-testid="card">
        <span data-testid="body">Contents</span>
      </Card>
    );

    const card = screen.getByTestId('card');
    expect(card).toHaveAttribute('data-slot', 'card');
    expect(card.className).toBe('overflow-hidden rounded-xl border border-line bg-surface');
    expect(screen.getByTestId('body')).toHaveTextContent('Contents');
  });

  it('renders the title as an h3 with its description', () => {
    render(
      <Card title="Routing" description="Where turns go.">
        <span data-testid="body">Contents</span>
      </Card>
    );

    expect(screen.getByRole('heading', { level: 3, name: 'Routing' })).toBeInTheDocument();
    expect(screen.getByText('Where turns go.')).toBeInTheDocument();
  });

  it('merges a caller className last-wins', () => {
    render(
      <Card data-testid="card" className="rounded-2xl">
        <span>Contents</span>
      </Card>
    );

    const card = screen.getByTestId('card');
    expect(card).toHaveClass('rounded-2xl');
    expect(card).not.toHaveClass('rounded-xl');
  });

  /* The three props below are additive. This is the guard that they stayed
     that way: with none of them passed the markup must be what it was before
     they existed — an unpadded, divided body and a flat heading block. */
  it('renders identically to the pre-prop Card when no new prop is passed', () => {
    const { container } = render(
      <Card title="Routing" description="Where turns go." data-testid="card">
        <span data-testid="body">Contents</span>
      </Card>
    );

    expect(container.innerHTML).toBe(
      '<div data-slot="card" data-testid="card" class="overflow-hidden rounded-xl border border-line bg-surface">' +
        '<div class="px-4 pb-0 pt-4">' +
        '<h3 class="text-xs font-semibold tracking-wide text-content-muted">Routing</h3>' +
        '<p class="mt-1 text-xs leading-relaxed text-content-muted">Where turns go.</p>' +
        '</div>' +
        '<div class="divide-y divide-line-subtle"><span data-testid="body">Contents</span></div>' +
        '</div>'
    );
  });

  describe('padded', () => {
    it('leaves the body unpadded by default', () => {
      render(
        <Card>
          <span data-testid="body">Contents</span>
        </Card>
      );

      expect(bodyOf()).not.toHaveClass('p-4');
    });

    it('pads the body when asked', () => {
      render(
        <Card padded>
          <span data-testid="body">Contents</span>
        </Card>
      );

      expect(bodyOf()).toHaveClass('p-4');
    });

    it('composes with divided={false} for a single-block card', () => {
      render(
        <Card padded divided={false}>
          <span data-testid="body">Contents</span>
        </Card>
      );

      expect(bodyOf().className).toBe('p-4');
    });
  });

  describe('divided', () => {
    it('divides the body by default', () => {
      render(
        <Card>
          <span data-testid="body">Contents</span>
        </Card>
      );

      expect(bodyOf()).toHaveClass('divide-y', 'divide-line-subtle');
    });

    it('drops the dividers when asked', () => {
      render(
        <Card divided={false}>
          <span data-testid="body">Contents</span>
        </Card>
      );

      const body = bodyOf();
      expect(body).not.toHaveClass('divide-y');
      expect(body).not.toHaveClass('divide-line-subtle');
    });
  });

  describe('headerRight', () => {
    it('is absent from the heading block by default', () => {
      render(
        <Card title="Seven-day cost">
          <span data-testid="body">Contents</span>
        </Card>
      );

      const title = screen.getByRole('heading', { level: 3, name: 'Seven-day cost' });
      // Flat heading block — the title's parent is the padded header itself,
      // not an interposed flex row.
      expect(title.parentElement).toHaveClass('px-4', 'pb-0', 'pt-4');
    });

    it('baseline-aligns the slot opposite the title', () => {
      render(
        <Card
          title="Seven-day cost"
          description="UTC days."
          headerRight={<span>times in UTC</span>}>
          <span data-testid="body">Contents</span>
        </Card>
      );

      const row = screen.getByText('times in UTC').parentElement!.parentElement!;
      expect(row).toHaveClass('flex', 'items-baseline', 'justify-between');
      expect(row).toHaveTextContent('Seven-day cost');
      expect(row).toHaveTextContent('UTC days.');
    });

    it('renders the heading block for a slot with no title', () => {
      render(
        <Card headerRight={<button type="button">View all</button>}>
          <span data-testid="body">Contents</span>
        </Card>
      );

      expect(screen.getByRole('button', { name: 'View all' })).toBeInTheDocument();
      expect(screen.queryByRole('heading')).not.toBeInTheDocument();
    });

    it('renders no heading block when neither a title nor a slot is given', () => {
      const { container } = render(
        <Card>
          <span data-testid="body">Contents</span>
        </Card>
      );

      expect(container.querySelector('[data-slot="card"]')!.children).toHaveLength(1);
    });
  });
});
