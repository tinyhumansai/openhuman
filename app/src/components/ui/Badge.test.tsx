import { render, screen } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { describe, expect, it, vi } from 'vitest';

import Badge, { type BadgeVariant, badgeVariants } from './Badge';

const VARIANTS: BadgeVariant[] = ['neutral', 'primary', 'success', 'warning', 'danger'];

describe('Badge', () => {
  it('renders the default badge exactly as before ...rest was forwarded', () => {
    render(<Badge data-testid="badge">Draft</Badge>);

    const badge = screen.getByTestId('badge');
    expect(badge.tagName).toBe('SPAN');
    expect(badge).toHaveTextContent('Draft');
    expect(badge).toHaveAttribute('data-slot', 'badge');
    expect(badge).toHaveAttribute('data-variant', 'neutral');
    // The class list is the variant recipe verbatim — no extra utilities crept in.
    expect(badge.getAttribute('class')).toBe(badgeVariants({ variant: undefined }));
    // Nothing beyond the four attributes the component has always emitted.
    expect(
      Array.from(badge.attributes)
        .map(attr => attr.name)
        .sort()
    ).toEqual(['class', 'data-slot', 'data-testid', 'data-variant']);
  });

  it.each(VARIANTS)('keeps the %s variant class list unchanged', variant => {
    render(
      <Badge data-testid="badge" variant={variant}>
        Label
      </Badge>
    );

    const badge = screen.getByTestId('badge');
    expect(badge).toHaveAttribute('data-variant', variant);
    expect(badge.getAttribute('class')).toBe(badgeVariants({ variant }));
  });

  it('merges className through cn() so a caller can override the recipe', () => {
    render(
      <Badge data-testid="badge" className="rounded-full">
        Pill
      </Badge>
    );

    const badge = screen.getByTestId('badge');
    expect(badge).toHaveClass('rounded-full');
    expect(badge).not.toHaveClass('rounded-md');
  });

  it('forwards an arbitrary span attribute to the DOM', () => {
    render(
      <Badge data-testid="badge" title="Delivered at 09:41" id="delivery-badge" lang="en">
        Sent
      </Badge>
    );

    const badge = screen.getByTestId('badge');
    // NotificationCard's pill carries a title tooltip; it must survive the migration.
    expect(badge).toHaveAttribute('title', 'Delivered at 09:41');
    expect(badge).toHaveAttribute('id', 'delivery-badge');
    expect(badge).toHaveAttribute('lang', 'en');
  });

  it('forwards aria attributes and event handlers', async () => {
    const onClick = vi.fn();
    render(
      <Badge data-testid="badge" aria-label="Two unread" role="status" onClick={onClick}>
        2
      </Badge>
    );

    const badge = screen.getByTestId('badge');
    expect(badge).toHaveAttribute('aria-label', 'Two unread');
    expect(badge).toHaveAttribute('role', 'status');

    await userEvent.click(badge);
    expect(onClick).toHaveBeenCalledTimes(1);
  });

  it('does not let a forwarded prop displace the component-owned attributes', () => {
    render(
      <Badge data-testid="badge" data-slot="not-a-badge" className="text-content-primary">
        Fixed
      </Badge>
    );

    expect(screen.getByTestId('badge')).toHaveAttribute('data-slot', 'badge');
  });
});
