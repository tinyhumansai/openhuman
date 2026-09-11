import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import { Alert, AlertDescription, AlertTitle, type AlertVariant, alertVariants } from './Alert';

const RAW_PALETTE = /\b(?:bg|text|border|ring)-(?:neutral|stone|slate|canvas|white|black)\b/;

const VARIANTS: AlertVariant[] = ['default', 'info', 'success', 'warning', 'destructive'];

/**
 * The class attribute every existing call site produced before `density`
 * existed. Every default-density assertion below compares against this, so a
 * later variant addition cannot silently reshape the ~34 alerts already in the
 * tree.
 */
const BASE_GEOMETRY = ['rounded-xl', 'px-4', 'py-3', 'text-sm'] as const;
const COMPACT_GEOMETRY = ['rounded-lg', 'px-3', 'py-2', 'text-xs'] as const;

describe('Alert', () => {
  it('renders its title and description', () => {
    render(
      <Alert data-testid="alert">
        <AlertTitle data-testid="title">Disk almost full</AlertTitle>
        <AlertDescription data-testid="description">Free some space.</AlertDescription>
      </Alert>
    );

    expect(screen.getByTestId('alert')).toHaveAttribute('data-slot', 'alert');
    expect(screen.getByTestId('title')).toHaveTextContent('Disk almost full');
    expect(screen.getByTestId('description')).toHaveTextContent('Free some space.');
    expect(screen.getByTestId('title')).toHaveAttribute('data-slot', 'alert-title');
    expect(screen.getByTestId('description')).toHaveAttribute('data-slot', 'alert-description');
  });

  it('defaults to the default variant', () => {
    render(<Alert data-testid="alert">Body</Alert>);

    expect(screen.getByTestId('alert')).toHaveAttribute('data-variant', 'default');
  });

  it.each(VARIANTS)('emits data-variant="%s"', variant => {
    render(
      <Alert variant={variant} data-testid="alert">
        Body
      </Alert>
    );

    expect(screen.getByTestId('alert')).toHaveAttribute('data-variant', variant);
  });

  it.each(['destructive', 'warning'] as const)('gives %s an assertive alert role', variant => {
    render(
      <Alert variant={variant} data-testid="alert">
        Body
      </Alert>
    );

    expect(screen.getByTestId('alert')).toHaveAttribute('role', 'alert');
  });

  it.each(['default', 'info', 'success'] as const)('leaves %s without an alert role', variant => {
    render(
      <Alert variant={variant} data-testid="alert">
        Body
      </Alert>
    );

    expect(screen.getByTestId('alert')).not.toHaveAttribute('role');
  });

  it('lets a caller override the role explicitly', () => {
    render(
      <Alert variant="info" role="status" data-testid="alert">
        Body
      </Alert>
    );

    expect(screen.getByTestId('alert')).toHaveAttribute('role', 'status');
  });

  it.each(['destructive', 'warning'] as const)(
    'lets %s trade its assertive role for a polite one',
    variant => {
      // `McpServerPanel`'s open-config failure: an error, but one that arrives
      // in response to a click and must not interrupt the reader.
      render(
        <Alert variant={variant} role="status" aria-live="polite" data-testid="alert">
          Body
        </Alert>
      );

      const el = screen.getByTestId('alert');
      expect(el).toHaveAttribute('role', 'status');
      expect(el).toHaveAttribute('aria-live', 'polite');
    }
  );

  it.each(['destructive', 'warning'] as const)(
    'drops the implicit role on a load-present %s notice',
    variant => {
      // The documented opt-out: presence of the prop decides, so an explicit
      // `undefined` removes the live region instead of falling back to it.
      render(
        <Alert variant={variant} role={undefined} data-testid="alert">
          Body
        </Alert>
      );

      expect(screen.getByTestId('alert')).not.toHaveAttribute('role');
    }
  );

  it('forwards rest props and a ref onto the DOM node', () => {
    let node: HTMLDivElement | null = null;
    render(
      <Alert
        ref={el => {
          node = el;
        }}
        id="disk-alert"
        aria-label="Disk"
        data-analytics-id="disk-alert"
        data-testid="alert">
        Body
      </Alert>
    );

    const el = screen.getByTestId('alert');
    expect(node).toBe(el);
    expect(el).toHaveAttribute('id', 'disk-alert');
    expect(el).toHaveAttribute('aria-label', 'Disk');
    expect(el).toHaveAttribute('data-analytics-id', 'disk-alert');
  });

  it('lets a caller className win over the defaults', () => {
    render(
      <Alert className="rounded-none" data-testid="alert">
        Body
      </Alert>
    );

    const cls = screen.getByTestId('alert').className;
    expect(cls).toContain('rounded-none');
    expect(cls).not.toContain('rounded-xl');
  });

  describe('density', () => {
    it.each(VARIANTS)(
      'renders %s identically with no density prop and with density="default"',
      variant => {
        // The guarantee the density variant was added under: every call site
        // that predates it keeps its exact class attribute.
        render(
          <Alert variant={variant} data-testid="implicit">
            Body
          </Alert>
        );
        render(
          <Alert variant={variant} density="default" data-testid="explicit">
            Body
          </Alert>
        );

        const implicit = screen.getByTestId('implicit');
        expect(implicit.className).toBe(screen.getByTestId('explicit').className);
        // …and that attribute is still what the cva base emits, unreordered.
        expect(implicit.className).toBe(alertVariants({ variant }));
        for (const utility of BASE_GEOMETRY) expect(implicit.className).toContain(utility);
        for (const utility of COMPACT_GEOMETRY) expect(implicit.className).not.toContain(utility);
      }
    );

    it.each(VARIANTS)('gives %s the dense geometry at density="compact"', variant => {
      render(
        <Alert variant={variant} density="compact" data-testid="alert">
          Body
        </Alert>
      );

      const cls = screen.getByTestId('alert').className;
      for (const utility of COMPACT_GEOMETRY) expect(cls).toContain(utility);
      // tailwind-merge must have dropped the base geometry, not stacked on it —
      // two competing paddings in one attribute is how the hand-rolled notices
      // ended up at four different sizes.
      for (const utility of BASE_GEOMETRY) expect(cls).not.toContain(utility);
    });

    it('keeps the tone classes when compacted', () => {
      render(
        <Alert variant="destructive" density="compact" data-testid="alert">
          Body
        </Alert>
      );

      const cls = screen.getByTestId('alert').className;
      expect(cls).toContain('border-coral-200');
      expect(cls).toContain('bg-coral-50');
      expect(cls).toContain('text-coral-600');
      expect(screen.getByTestId('alert')).toHaveAttribute('role', 'alert');
    });

    it('lets a caller className win over the compact geometry too', () => {
      render(
        <Alert density="compact" className="px-6" data-testid="alert">
          Body
        </Alert>
      );

      const cls = screen.getByTestId('alert').className;
      expect(cls).toContain('px-6');
      expect(cls).not.toContain('px-3');
      expect(cls).toContain('py-2');
    });

    it('shrinks the description with the box', () => {
      render(
        <Alert density="compact" data-testid="alert">
          <AlertTitle data-testid="title">Title</AlertTitle>
          <AlertDescription data-testid="description">Description</AlertDescription>
        </Alert>
      );

      const description = screen.getByTestId('description');
      expect(description.className).toContain('text-xs');
      expect(description.className).not.toContain('text-sm');
      expect(description.className).toContain('leading-relaxed');
    });

    it('leaves the description at text-sm inside a default alert', () => {
      render(
        <Alert data-testid="alert">
          <AlertDescription data-testid="description">Description</AlertDescription>
        </Alert>
      );

      // Byte-identical to what the primitive emitted before `density` existed.
      expect(screen.getByTestId('description').className).toBe(
        'text-sm leading-relaxed opacity-90'
      );
    });

    it('leaves a standalone description at text-sm', () => {
      render(<AlertDescription data-testid="description">Description</AlertDescription>);

      expect(screen.getByTestId('description').className).toBe(
        'text-sm leading-relaxed opacity-90'
      );
    });

    it('lets a description className win over the inherited density', () => {
      render(
        <Alert density="compact" data-testid="alert">
          <AlertDescription className="text-base" data-testid="description">
            Description
          </AlertDescription>
        </Alert>
      );

      const cls = screen.getByTestId('description').className;
      expect(cls).toContain('text-base');
      expect(cls).not.toContain('text-xs');
    });

    it('does not change the role derivation', () => {
      render(
        <Alert variant="info" density="compact" data-testid="info">
          Body
        </Alert>
      );
      render(
        <Alert variant="warning" density="compact" data-testid="warning">
          Body
        </Alert>
      );

      expect(screen.getByTestId('info')).not.toHaveAttribute('role');
      expect(screen.getByTestId('warning')).toHaveAttribute('role', 'alert');
    });

    it.each(VARIANTS)('keeps %s on design tokens when compacted', variant => {
      render(
        <Alert variant={variant} density="compact" data-testid="alert">
          <AlertDescription data-testid="description">Description</AlertDescription>
        </Alert>
      );

      expect(screen.getByTestId('alert').className).not.toMatch(RAW_PALETTE);
      expect(screen.getByTestId('description').className).not.toMatch(RAW_PALETTE);
    });
  });

  it.each(VARIANTS)('resolves %s to design tokens, never a raw palette class', variant => {
    render(
      <Alert variant={variant} data-testid="alert">
        <AlertTitle data-testid="title">Title</AlertTitle>
        <AlertDescription data-testid="description">Description</AlertDescription>
      </Alert>
    );

    expect(screen.getByTestId('alert').className).not.toMatch(RAW_PALETTE);
    expect(screen.getByTestId('title').className).not.toMatch(RAW_PALETTE);
    expect(screen.getByTestId('description').className).not.toMatch(RAW_PALETTE);
  });
});
