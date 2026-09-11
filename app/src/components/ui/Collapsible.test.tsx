import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import {
  CollapsibleContent,
  CollapsibleRoot,
  type CollapsibleSize,
  CollapsibleTrigger,
  type CollapsibleVariant,
} from './Collapsible';

const RAW_PALETTE = /\b(?:bg|text|border|ring)-(?:neutral|stone|slate|canvas|white|black)\b/;

const renderCollapsible = (props?: {
  variant?: CollapsibleVariant;
  size?: CollapsibleSize;
  defaultOpen?: boolean;
}) =>
  render(
    <CollapsibleRoot
      variant={props?.variant}
      defaultOpen={props?.defaultOpen}
      data-testid="collapsible">
      <CollapsibleTrigger size={props?.size} data-testid="trigger">
        Details
      </CollapsibleTrigger>
      <CollapsibleContent size={props?.size} data-testid="content">
        Body
      </CollapsibleContent>
    </CollapsibleRoot>
  );

describe('Collapsible', () => {
  it('renders closed by default', () => {
    renderCollapsible();

    expect(screen.getByTestId('trigger')).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByText('Body')).toBeNull();
  });

  it('renders open when defaultOpen is set', () => {
    renderCollapsible({ defaultOpen: true });

    expect(screen.getByTestId('trigger')).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('Body')).toBeInTheDocument();
  });

  it('toggles from the keyboard', async () => {
    const user = userEvent.setup();
    renderCollapsible();
    const trigger = screen.getByTestId('trigger');

    trigger.focus();
    await user.keyboard('{Enter}');
    expect(trigger).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('Body')).toBeInTheDocument();

    await user.keyboard(' ');
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByText('Body')).toBeNull();
  });

  it('reports open state through onOpenChange', () => {
    const onOpenChange = vi.fn();
    render(
      <CollapsibleRoot onOpenChange={onOpenChange}>
        <CollapsibleTrigger data-testid="trigger">Details</CollapsibleTrigger>
        <CollapsibleContent>Body</CollapsibleContent>
      </CollapsibleRoot>
    );

    fireEvent.click(screen.getByTestId('trigger'));
    expect(onOpenChange).toHaveBeenCalledWith(true);
  });

  it('emits the data-slot / data-variant / data-size contract', () => {
    renderCollapsible({ variant: 'card', size: 'sm', defaultOpen: true });

    const root = screen.getByTestId('collapsible');
    expect(root).toHaveAttribute('data-slot', 'collapsible');
    expect(root).toHaveAttribute('data-variant', 'card');
    expect(root).toHaveAttribute('data-state', 'open');

    const trigger = screen.getByTestId('trigger');
    expect(trigger).toHaveAttribute('data-slot', 'collapsible-trigger');
    expect(trigger).toHaveAttribute('data-size', 'sm');

    const content = screen.getByTestId('content');
    expect(content).toHaveAttribute('data-slot', 'collapsible-content');
    expect(content).toHaveAttribute('data-size', 'sm');
    expect(content).toHaveAttribute('data-state', 'open');
  });

  it('defaults data-variant and data-size when the axes are omitted', () => {
    renderCollapsible({ defaultOpen: true });

    expect(screen.getByTestId('collapsible')).toHaveAttribute('data-variant', 'plain');
    expect(screen.getByTestId('trigger')).toHaveAttribute('data-size', 'md');
    expect(screen.getByTestId('content')).toHaveAttribute('data-size', 'md');
  });

  it('forwards ...rest and data-testid onto the underlying DOM nodes', () => {
    const onClick = vi.fn();
    render(
      <CollapsibleRoot defaultOpen data-testid="root" id="panel">
        <CollapsibleTrigger data-testid="trigger" onClick={onClick} title="toggle">
          Details
        </CollapsibleTrigger>
        <CollapsibleContent data-testid="content" aria-label="details body">
          Body
        </CollapsibleContent>
      </CollapsibleRoot>
    );

    expect(screen.getByTestId('root')).toHaveAttribute('id', 'panel');
    expect(screen.getByTestId('trigger')).toHaveAttribute('title', 'toggle');
    expect(screen.getByTestId('content')).toHaveAttribute('aria-label', 'details body');

    fireEvent.click(screen.getByTestId('trigger'));
    expect(onClick).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('trigger')).toHaveAttribute('aria-expanded', 'false');
  });

  it('forwards a ref to the trigger element', () => {
    let node: HTMLElement | null = null;
    render(
      <CollapsibleRoot>
        <CollapsibleTrigger
          ref={(el: HTMLButtonElement | null) => {
            node = el;
          }}>
          Details
        </CollapsibleTrigger>
        <CollapsibleContent>Body</CollapsibleContent>
      </CollapsibleRoot>
    );

    expect(node).not.toBeNull();
    expect((node as unknown as HTMLElement).tagName).toBe('BUTTON');
  });

  it('lets a caller className win over the default', () => {
    render(
      <CollapsibleRoot variant="card" data-testid="root" className="rounded-none">
        <CollapsibleTrigger>Details</CollapsibleTrigger>
        <CollapsibleContent>Body</CollapsibleContent>
      </CollapsibleRoot>
    );

    const root = screen.getByTestId('root');
    expect(root.className).toContain('rounded-none');
    expect(root.className).not.toMatch(/\brounded-xl\b/);
  });

  /** Same themeability contract as the rest of the primitive layer. */
  it('never renders a raw palette colour in any variant/size combination', () => {
    const variants = ['plain', 'card'] as const;
    const sizes = ['sm', 'md', 'lg'] as const;

    for (const variant of variants) {
      for (const size of sizes) {
        const { container, unmount } = renderCollapsible({ variant, size, defaultOpen: true });
        for (const el of Array.from(container.querySelectorAll<HTMLElement>('[class]'))) {
          expect(el.className.toString()).not.toMatch(RAW_PALETTE);
        }
        unmount();
      }
    }
  });
});
