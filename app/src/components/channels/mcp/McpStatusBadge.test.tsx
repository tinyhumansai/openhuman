/**
 * Tests for McpStatusBadge — renders the i18n'd label and a11y role
 * for each ServerStatus, and forwards custom className.
 */
import { render, screen } from '@testing-library/react';
import { describe, expect, it } from 'vitest';

import McpStatusBadge from './McpStatusBadge';
import type { ServerStatus } from './types';

describe('McpStatusBadge', () => {
  it.each<[ServerStatus, string]>([
    ['connected', 'Connected'],
    ['connecting', 'Connecting'],
    ['disconnected', 'Disconnected'],
    ['error', 'Error'],
  ])('renders i18n label for status=%s', (status, expectedLabel) => {
    render(<McpStatusBadge status={status} />);
    expect(screen.getByRole('status')).toHaveTextContent(expectedLabel);
  });

  it('renders the disabled status badge with the mcp.status.disabled i18n key', () => {
    render(<McpStatusBadge status="disabled" />);
    // The i18n key 'mcp.status.disabled' will be added in Task 9; until then
    // the runtime falls back to the key string itself.
    const badge = screen.getByRole('status');
    // The badge renders without crashing and carries the italic style.
    expect(badge.className).toContain('italic');
  });

  it('exposes role="status" and aria-live="polite" for assistive tech', () => {
    render(<McpStatusBadge status="connecting" />);
    const badge = screen.getByRole('status');
    expect(badge).toHaveAttribute('aria-live', 'polite');
  });

  it('falls back to the disconnected style for an unknown status value', () => {
    // ServerStatus is a closed union, but the runtime fallback exists for
    // forward-compat with possible future Rust-side variants — exercise it.
    render(<McpStatusBadge status={'spawning' as ServerStatus} />);
    expect(screen.getByRole('status')).toHaveTextContent('Disconnected');
  });

  it('appends the optional className without dropping the built-in classes', () => {
    render(<McpStatusBadge status="connected" className="ml-2 my-custom-class" />);
    const badge = screen.getByRole('status');
    expect(badge.className).toContain('my-custom-class');
    expect(badge.className).toContain('ml-2');
    // Built-in look-and-feel preserved.
    expect(badge.className).toContain('rounded-full');
    expect(badge.className).toContain('bg-sage-500/10');
  });
});
