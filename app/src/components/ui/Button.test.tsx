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
    expect(btn.className).toMatch(/border-neutral-300/);
    expect(btn.className).toMatch(/bg-neutral-0/);
  });

  it('applies ghost variant classes', () => {
    render(<Button variant="ghost">Skip</Button>);
    const btn = screen.getByRole('button', { name: 'Skip' });
    expect(btn.className).toMatch(/bg-transparent/);
    expect(btn.className).toMatch(/text-neutral-700/);
  });

  it('applies danger variant classes', () => {
    render(<Button variant="danger">Delete</Button>);
    const btn = screen.getByRole('button', { name: 'Delete' });
    expect(btn.className).toMatch(/text-coral-600/);
    expect(btn.className).toMatch(/hover:bg-coral-50/);
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
});
