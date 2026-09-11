import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it } from 'vitest';

import { Source, Sources, SourcesContent, SourcesTrigger } from './Sources';

const RAW_PALETTE =
  /\b(?:bg|text|border|ring)-(?:neutral|stone|slate|zinc|gray|canvas|white|black)\b/;

const collectClasses = (root: HTMLElement) =>
  [root, ...Array.from(root.querySelectorAll<HTMLElement>('*'))]
    .map(el => el.getAttribute('class') ?? '')
    .join(' ');

const renderSources = (props?: { defaultOpen?: boolean }) =>
  render(
    <Sources data-testid="sources" defaultOpen={props?.defaultOpen}>
      <SourcesTrigger count={2} data-testid="sources-trigger" />
      <SourcesContent data-testid="sources-content">
        <Source href="https://example.com" title="Example" data-testid="source" />
      </SourcesContent>
    </Sources>
  );

describe('Sources', () => {
  it('renders the trigger with the source count and stays closed by default', () => {
    renderSources();

    const trigger = screen.getByTestId('sources-trigger');
    expect(trigger).toHaveTextContent('Used 2 sources');
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByTestId('source')).toBeNull();
  });

  it('discloses the sources when the trigger is clicked', async () => {
    const user = userEvent.setup();
    renderSources();

    await user.click(screen.getByTestId('sources-trigger'));

    const link = screen.getByTestId('source');
    expect(link).toHaveAttribute('href', 'https://example.com');
    expect(link).toHaveAttribute('target', '_blank');
    expect(link).toHaveAttribute('rel', 'noreferrer');
    expect(link).toHaveTextContent('Example');
  });

  it('renders custom trigger and source children instead of the defaults', () => {
    render(
      <Sources defaultOpen>
        <SourcesTrigger count={9} data-testid="trigger">
          <span>Custom trigger</span>
        </SourcesTrigger>
        <SourcesContent>
          <Source href="https://example.com" title="Example" data-testid="source">
            <span>Custom source</span>
          </Source>
        </SourcesContent>
      </Sources>
    );

    expect(screen.getByTestId('trigger')).toHaveTextContent('Custom trigger');
    expect(screen.getByTestId('trigger')).not.toHaveTextContent('Used 9 sources');
    expect(screen.getByTestId('source')).toHaveTextContent('Custom source');
  });

  it('passes rest props through and emits the data-slot contract', () => {
    renderSources({ defaultOpen: true });

    expect(screen.getByTestId('sources')).toHaveAttribute('data-slot', 'sources');
    expect(screen.getByTestId('sources-trigger')).toHaveAttribute('data-slot', 'sources-trigger');
    expect(screen.getByTestId('sources-content')).toHaveAttribute('data-slot', 'sources-content');
    expect(screen.getByTestId('source')).toHaveAttribute('data-slot', 'source');
  });

  it('uses design tokens, never raw palette classes', () => {
    const { container } = renderSources({ defaultOpen: true });

    expect(collectClasses(container)).not.toMatch(RAW_PALETTE);
  });
});
