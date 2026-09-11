import { fireEvent, render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import {
  AccordionContent,
  AccordionItem,
  AccordionRoot,
  type AccordionSize,
  AccordionTrigger,
  type AccordionVariant,
} from './Accordion';

const RAW_PALETTE = /\b(?:bg|text|border|ring)-(?:neutral|stone|slate|canvas|white|black)\b/;

const renderAccordion = (props?: {
  variant?: AccordionVariant;
  size?: AccordionSize;
  defaultValue?: string;
}) =>
  render(
    <AccordionRoot
      type="single"
      collapsible
      variant={props?.variant}
      defaultValue={props?.defaultValue}
      data-testid="accordion">
      <AccordionItem value="one" variant={props?.variant} data-testid="item-one">
        <AccordionTrigger size={props?.size} data-testid="trigger-one">
          First
        </AccordionTrigger>
        <AccordionContent size={props?.size} data-testid="content-one">
          First body
        </AccordionContent>
      </AccordionItem>
      <AccordionItem value="two" variant={props?.variant}>
        <AccordionTrigger size={props?.size}>Second</AccordionTrigger>
        <AccordionContent size={props?.size}>Second body</AccordionContent>
      </AccordionItem>
    </AccordionRoot>
  );

describe('Accordion', () => {
  it('renders its triggers collapsed by default', () => {
    renderAccordion();

    const trigger = screen.getByRole('button', { name: /First/ });
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
    expect(screen.queryByText('First body')).toBeNull();
  });

  it('opens the matching section when a trigger is activated', () => {
    renderAccordion();

    fireEvent.click(screen.getByRole('button', { name: /First/ }));

    expect(screen.getByRole('button', { name: /First/ })).toHaveAttribute('aria-expanded', 'true');
    expect(screen.getByText('First body')).toBeInTheDocument();
    // `type="single"` exclusivity: the sibling stays shut.
    expect(screen.queryByText('Second body')).toBeNull();
  });

  it('toggles a section open and shut from the keyboard', async () => {
    const user = userEvent.setup();
    renderAccordion();
    const trigger = screen.getByRole('button', { name: /First/ });

    trigger.focus();
    await user.keyboard('{Enter}');
    expect(trigger).toHaveAttribute('aria-expanded', 'true');

    // `collapsible` on the root is what allows the open section to shut again.
    await user.keyboard(' ');
    expect(trigger).toHaveAttribute('aria-expanded', 'false');
  });

  it('moves focus between headers with the arrow keys', () => {
    renderAccordion();
    const first = screen.getByRole('button', { name: /First/ });
    const second = screen.getByRole('button', { name: /Second/ });

    first.focus();
    fireEvent.keyDown(first, { key: 'ArrowDown' });
    expect(second).toHaveFocus();

    fireEvent.keyDown(second, { key: 'ArrowUp' });
    expect(first).toHaveFocus();
  });

  it('emits the data-slot / data-variant / data-size contract', () => {
    renderAccordion({ variant: 'card', size: 'lg', defaultValue: 'one' });

    expect(screen.getByTestId('accordion')).toHaveAttribute('data-slot', 'accordion');
    expect(screen.getByTestId('accordion')).toHaveAttribute('data-variant', 'card');
    expect(screen.getByTestId('item-one')).toHaveAttribute('data-slot', 'accordion-item');
    expect(screen.getByTestId('item-one')).toHaveAttribute('data-variant', 'card');

    const trigger = screen.getByTestId('trigger-one');
    expect(trigger).toHaveAttribute('data-slot', 'accordion-trigger');
    expect(trigger).toHaveAttribute('data-size', 'lg');

    const content = screen.getByTestId('content-one');
    expect(content).toHaveAttribute('data-slot', 'accordion-content');
    expect(content).toHaveAttribute('data-size', 'lg');
    expect(content).toHaveAttribute('data-state', 'open');
  });

  it('defaults data-variant and data-size when the axes are omitted', () => {
    renderAccordion({ defaultValue: 'one' });

    expect(screen.getByTestId('accordion')).toHaveAttribute('data-variant', 'plain');
    expect(screen.getByTestId('trigger-one')).toHaveAttribute('data-size', 'md');
    expect(screen.getByTestId('content-one')).toHaveAttribute('data-size', 'md');
  });

  it('forwards ...rest and data-testid onto the underlying DOM nodes', () => {
    const onClick = vi.fn();
    render(
      <AccordionRoot type="multiple" data-testid="root" id="faq">
        <AccordionItem value="a" data-testid="item" id="faq-a">
          <AccordionTrigger data-testid="trigger" onClick={onClick} title="open me">
            Q
          </AccordionTrigger>
          <AccordionContent data-testid="content" forceMount>
            A
          </AccordionContent>
        </AccordionItem>
      </AccordionRoot>
    );

    expect(screen.getByTestId('root')).toHaveAttribute('id', 'faq');
    expect(screen.getByTestId('item')).toHaveAttribute('id', 'faq-a');
    expect(screen.getByTestId('trigger')).toHaveAttribute('title', 'open me');
    expect(screen.getByTestId('content')).toBeInTheDocument();

    fireEvent.click(screen.getByTestId('trigger'));
    // The caller's handler runs alongside the primitive's own toggle.
    expect(onClick).toHaveBeenCalledTimes(1);
    expect(screen.getByTestId('trigger')).toHaveAttribute('aria-expanded', 'true');
  });

  it('forwards a ref to the trigger element', () => {
    let node: HTMLElement | null = null;
    render(
      <AccordionRoot type="single" collapsible>
        <AccordionItem value="a">
          <AccordionTrigger
            ref={(el: HTMLButtonElement | null) => {
              node = el;
            }}>
            Q
          </AccordionTrigger>
          <AccordionContent>A</AccordionContent>
        </AccordionItem>
      </AccordionRoot>
    );

    expect(node).not.toBeNull();
    expect((node as unknown as HTMLElement).tagName).toBe('BUTTON');
  });

  it('lets a caller className win over the default', () => {
    render(
      <AccordionRoot type="single" collapsible data-testid="root" className="w-1/2">
        <AccordionItem value="a">
          <AccordionTrigger>Q</AccordionTrigger>
          <AccordionContent>A</AccordionContent>
        </AccordionItem>
      </AccordionRoot>
    );

    const root = screen.getByTestId('root');
    expect(root.className).toContain('w-1/2');
    expect(root.className).not.toMatch(/\bw-full\b/);
  });

  it('hides the chevron when asked, and marks it aria-hidden otherwise', () => {
    const { container, unmount } = render(
      <AccordionRoot type="single" collapsible>
        <AccordionItem value="a">
          <AccordionTrigger>Q</AccordionTrigger>
          <AccordionContent>A</AccordionContent>
        </AccordionItem>
      </AccordionRoot>
    );
    const chevron = container.querySelector('[data-slot="accordion-chevron"]');
    expect(chevron).not.toBeNull();
    expect(chevron).toHaveAttribute('aria-hidden', 'true');
    unmount();

    const bare = render(
      <AccordionRoot type="single" collapsible>
        <AccordionItem value="a">
          <AccordionTrigger showChevron={false}>Q</AccordionTrigger>
          <AccordionContent>A</AccordionContent>
        </AccordionItem>
      </AccordionRoot>
    );
    expect(bare.container.querySelector('[data-slot="accordion-chevron"]')).toBeNull();
  });

  /**
   * The themeability contract: every variant/size combination must resolve to
   * semantic tokens, which follow a user's theme, never a raw palette scale,
   * which does not. Asserted on the resolved className, not on a frozen class
   * string, so restyling is free but de-tokenising is not.
   */
  it('never renders a raw palette colour in any variant/size combination', () => {
    const variants = ['plain', 'card', 'contained'] as const;
    const sizes = ['sm', 'md', 'lg'] as const;

    for (const variant of variants) {
      for (const size of sizes) {
        const { container, unmount } = renderAccordion({ variant, size, defaultValue: 'one' });
        for (const el of Array.from(container.querySelectorAll<HTMLElement>('[class]'))) {
          expect(el.className.toString()).not.toMatch(RAW_PALETTE);
        }
        unmount();
      }
    }
  });
});
