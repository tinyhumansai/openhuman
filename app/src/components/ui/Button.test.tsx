import { render, screen } from '@testing-library/react';
import { describe, expect, it, vi } from 'vitest';

import Button from './Button';

describe('Button', () => {
  it('renders children and defaults to primary/md with type=button', () => {
    render(<Button>Send</Button>);
    const btn = screen.getByRole('button', { name: 'Send' });
    expect(btn).toBeInTheDocument();
    expect(btn).toHaveAttribute('type', 'button');
    expect(btn.className).toMatch(/bg-primary-500/);
    expect(btn.className).toMatch(/h-9/);
    expect(btn.className).toMatch(/focus-visible:ring-primary-500\/25/);
  });

  it('applies secondary variant classes', () => {
    render(<Button variant="secondary">Cancel</Button>);
    const btn = screen.getByRole('button', { name: 'Cancel' });
    // Migrated to semantic surface/line tokens (themeable).
    expect(btn.className).toMatch(/border-line-strong/);
    expect(btn.className).toMatch(/bg-surface/);
  });

  it('applies tertiary variant classes', () => {
    render(<Button variant="tertiary">Skip</Button>);
    const btn = screen.getByRole('button', { name: 'Skip' });
    expect(btn.className).toMatch(/bg-transparent/);
    expect(btn.className).toMatch(/text-content-secondary/);
  });

  it('renders a filled coral button for primary + tone="danger"', () => {
    render(
      <Button variant="primary" tone="danger">
        Delete
      </Button>
    );
    const btn = screen.getByRole('button', { name: 'Delete' });
    expect(btn.className).toMatch(/bg-coral-500/);
    expect(btn.className).toMatch(/text-content-inverted/);
  });

  it('renders a coral outline-solid for secondary + tone="danger"', () => {
    render(
      <Button variant="secondary" tone="danger">
        Remove
      </Button>
    );
    const btn = screen.getByRole('button', { name: 'Remove' });
    expect(btn.className).toMatch(/text-coral-600/);
    expect(btn.className).toMatch(/border-coral-300\/50/);
    expect(btn.className).toMatch(/hover:bg-coral-50/);
  });

  it('renders coral text for tertiary + tone="danger"', () => {
    render(
      <Button variant="tertiary" tone="danger">
        Discard
      </Button>
    );
    const btn = screen.getByRole('button', { name: 'Discard' });
    expect(btn.className).toMatch(/text-coral-600/);
    expect(btn.className).not.toMatch(/border-coral/);
  });

  it('squares the footprint for iconOnly (no horizontal padding)', () => {
    render(
      <Button iconOnly size="md" aria-label="Close">
        <span data-testid="icon" />
      </Button>
    );
    const btn = screen.getByRole('button', { name: 'Close' });
    expect(btn.className).toMatch(/h-9/);
    expect(btn.className).toMatch(/w-9/);
    expect(btn.className).not.toMatch(/px-4/);
  });

  it('honors size=xl classes', () => {
    render(
      <Button size="xl" variant="primary">
        Open
      </Button>
    );
    const btn = screen.getByRole('button', { name: 'Open' });
    expect(btn.className).toMatch(/h-14/);
    expect(btn.className).toMatch(/rounded-xl/);
  });

  it('honors size=xs classes', () => {
    render(<Button size="xs">tiny</Button>);
    const btn = screen.getByRole('button', { name: 'tiny' });
    expect(btn.className).toMatch(/h-6/);
    expect(btn.className).toMatch(/text-xs/);
  });

  it('merges extra className', () => {
    render(<Button className="w-full">Wide</Button>);
    const btn = screen.getByRole('button', { name: 'Wide' });
    expect(btn.className).toMatch(/w-full/);
  });

  it('exposes a stable analytics identifier without forwarding the prop name', () => {
    render(<Button analyticsId="flows-run">Run</Button>);
    const btn = screen.getByRole('button', { name: 'Run' });
    expect(btn).toHaveAttribute('data-analytics-id', 'flows-run');
    expect(btn).not.toHaveAttribute('analyticsId');
  });

  it('respects disabled: does not fire onClick and has disabled attr', () => {
    const onClick = vi.fn();
    render(
      <Button disabled onClick={onClick}>
        No
      </Button>
    );
    const btn = screen.getByRole('button', { name: 'No' });
    expect(btn).toBeDisabled();
    btn.click();
    expect(onClick).not.toHaveBeenCalled();
    expect(btn.className).toMatch(/disabled:opacity-40/);
  });

  it('fires onClick when enabled', () => {
    const onClick = vi.fn();
    render(<Button onClick={onClick}>Go</Button>);
    screen.getByRole('button', { name: 'Go' }).click();
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('renders leading and trailing icons', () => {
    render(
      <Button leadingIcon={<span data-testid="lead" />} trailingIcon={<span data-testid="trail" />}>
        Label
      </Button>
    );
    expect(screen.getByTestId('lead')).toBeInTheDocument();
    expect(screen.getByTestId('trail')).toBeInTheDocument();
  });

  /**
   * cva CONCATENATES compound variants where the previous lookup-table
   * implementation SUBSTITUTED. A danger button's raw cva output therefore
   * contains the default variant's background as well as coral, and only
   * `cn()`'s tailwind-merge pass drops the loser.
   *
   * Asserting the winning class is present would pass even if the merge were
   * broken and both classes shipped — CSS order, not the class list, would then
   * decide the colour. So these assert the LOSING class is absent.
   */
  describe('danger tone overrides the base variant rather than doubling up', () => {
    it.each([
      ['primary', /bg-primary-500/, /bg-coral-500/],
      ['secondary', /border-line-strong/, /border-coral-300/],
      ['tertiary', /text-content-secondary/, /text-coral-600/],
    ] as const)('drops the default %s surface', (variant, losing, winning) => {
      render(
        <Button variant={variant} tone="danger">
          Delete
        </Button>
      );
      const btn = screen.getByRole('button', { name: 'Delete' });
      expect(btn.className).toMatch(winning);
      expect(btn.className).not.toMatch(losing);
    });
  });

  it('lets a caller className win over the variant default', () => {
    render(<Button className="bg-sage-500">Go</Button>);
    const btn = screen.getByRole('button', { name: 'Go' });
    expect(btn.className).toMatch(/bg-sage-500/);
    expect(btn.className).not.toMatch(/bg-primary-500/);
  });

  it('exposes variant state as data attributes for tests and styling hooks', () => {
    render(
      <Button variant="secondary" tone="danger" size="lg">
        Remove
      </Button>
    );
    const btn = screen.getByRole('button', { name: 'Remove' });
    expect(btn).toHaveAttribute('data-slot', 'button');
    expect(btn).toHaveAttribute('data-variant', 'secondary');
    expect(btn).toHaveAttribute('data-tone', 'danger');
    expect(btn).toHaveAttribute('data-size', 'lg');
  });

  it('forwards data-testid so E2E selectors survive the primitive swap', () => {
    render(<Button data-testid="save-btn">Save</Button>);
    expect(screen.getByTestId('save-btn')).toBeInTheDocument();
  });

  // `Slot` merges props onto the child, and the delegated analytics listener in
  // services/analyticsInteractions.ts reads the attribute off whatever element
  // actually receives the click — so it has to survive both paths.
  it('emits data-analytics-id as a plain button', () => {
    render(<Button analyticsId="settings.save">Save</Button>);
    expect(screen.getByRole('button', { name: 'Save' })).toHaveAttribute(
      'data-analytics-id',
      'settings.save'
    );
  });

  it('emits data-analytics-id through asChild onto the rendered child', () => {
    render(
      <Button asChild analyticsId="settings.docs">
        <a href="https://example.test">Docs</a>
      </Button>
    );
    const link = screen.getByRole('link', { name: 'Docs' });
    expect(link).toHaveAttribute('data-analytics-id', 'settings.docs');
    expect(link).not.toHaveAttribute('type');
  });

  /**
   * The themeability contract, expressed once instead of as frozen class
   * strings: every variant must be built from semantic/accent tokens, which
   * follow a user's theme, never from a raw palette scale, which does not.
   */
  it('never renders a raw palette colour in any variant combination', () => {
    const RAW_PALETTE = /\b(?:bg|text|border|ring)-(?:neutral|stone|slate|canvas|white|black)\b/;
    const variants = ['primary', 'secondary', 'tertiary'] as const;
    const tones = ['default', 'danger'] as const;
    const sizes = ['xs', 'sm', 'md', 'lg', 'xl'] as const;

    for (const variant of variants) {
      for (const tone of tones) {
        for (const size of sizes) {
          for (const iconOnly of [false, true]) {
            const { unmount } = render(
              <Button variant={variant} tone={tone} size={size} iconOnly={iconOnly}>
                x
              </Button>
            );
            const className = screen.getByRole('button').className;
            expect(className, `${variant}/${tone}/${size}`).not.toMatch(RAW_PALETTE);
            unmount();
          }
        }
      }
    }
  });
});
