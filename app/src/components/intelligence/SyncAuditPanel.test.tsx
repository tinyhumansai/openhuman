/**
 * Vitest for `<SyncAuditPanel />` (issue #3116 — coverage for the sync-audit
 * history surface shipped in PR #3113).
 *
 * Covers:
 * - loading → loaded transition (renders rows once the audit log resolves)
 * - formatting of tokens (k/M), cost ($x.xxxx), and duration (ms / s / m s)
 * - the scope label mapping for github / gmail / rebuild scopes
 * - success ✓ vs failure ✗ status glyphs
 * - the empty state when no runs are recorded
 *
 * Only the `memorySyncAuditLog` wrapper is swapped for a spy; everything
 * else in the `tauriCommands` barrel (types, sibling wrappers) is inherited
 * verbatim so the panel sees the production module shape.
 */
import { render, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { SyncAuditEntry } from '../../utils/tauriCommands';
import { SyncAuditPanel, timeAgo } from './SyncAuditPanel';

const mockAuditLog = vi.fn();

vi.mock('../../utils/tauriCommands', async importOriginal => {
  const actual = await importOriginal<typeof import('../../utils/tauriCommands')>();
  return { ...actual, memorySyncAuditLog: (...args: unknown[]) => mockAuditLog(...args) };
});

function entry(overrides: Partial<SyncAuditEntry> = {}): SyncAuditEntry {
  return {
    timestamp: new Date().toISOString(),
    source_id: 'src-1',
    source_kind: 'github_repo',
    scope: 'github:tinyhumansai/openhuman',
    items_fetched: 12,
    batches: 1,
    input_tokens: 1500,
    output_tokens: 500,
    estimated_cost_usd: 0.0123,
    duration_ms: 4200,
    success: true,
    ...overrides,
  };
}

describe('<SyncAuditPanel />', () => {
  beforeEach(() => {
    mockAuditLog.mockReset();
  });

  it('renders the empty state when there are no runs', async () => {
    mockAuditLog.mockResolvedValue([]);
    render(<SyncAuditPanel />);
    expect(await screen.findByText('No sync runs recorded yet.')).toBeInTheDocument();
  });

  it('renders entries once the audit log resolves', async () => {
    mockAuditLog.mockResolvedValue([entry()]);
    render(<SyncAuditPanel />);

    // Summary line: "1 sync runs" (count + label render as sibling text nodes
    // inside one span, so match the span's normalized text content).
    expect(await screen.findByText(/^\s*1\s+sync runs\s*$/)).toBeInTheDocument();
    // The github scope is rendered through scopeLabel().
    expect(screen.getByText('GitHub · tinyhumansai/openhuman')).toBeInTheDocument();
    // Item count cell.
    expect(screen.getByText('12')).toBeInTheDocument();
  });

  it('formats tokens, cost, and duration for each row', async () => {
    mockAuditLog.mockResolvedValue([
      entry({
        input_tokens: 1_500, // 1.5k in
        output_tokens: 500, // combined 2.0k displayed
        estimated_cost_usd: 0.0123,
        duration_ms: 4200, // 4.2s
      }),
    ]);
    render(<SyncAuditPanel />);

    // formatTokens(input+output) → 2.0k
    expect(await screen.findByText('2.0k')).toBeInTheDocument();
    // cost → $0.0123 (4 dp)
    expect(screen.getByText('$0.0123')).toBeInTheDocument();
    // duration → 4.2s
    expect(screen.getByText('4.2s')).toBeInTheDocument();
  });

  it('formats sub-second, minute, and millions correctly', async () => {
    mockAuditLog.mockResolvedValue([
      entry({
        source_id: 'big',
        scope: 'gmail:test-at-example-dot-com',
        input_tokens: 1_500_000, // 1.5M in
        output_tokens: 500_000, // combined 2.0M
        duration_ms: 125_000, // 2m 5s
        estimated_cost_usd: 1.5,
      }),
    ]);
    render(<SyncAuditPanel />);

    expect(await screen.findByText('2.0M')).toBeInTheDocument();
    expect(screen.getByText('2m 5s')).toBeInTheDocument();
    // gmail scope label de-slugifies the email.
    expect(screen.getByText('Gmail · test@example.com')).toBeInTheDocument();
  });

  it('aggregates totals across multiple runs', async () => {
    mockAuditLog.mockResolvedValue([
      entry({ source_id: 'a', estimated_cost_usd: 0.01, input_tokens: 1000, output_tokens: 0 }),
      entry({ source_id: 'b', estimated_cost_usd: 0.02, input_tokens: 1000, output_tokens: 0 }),
    ]);
    render(<SyncAuditPanel />);

    // "2 sync runs"
    expect(await screen.findByText(/^\s*2\s+sync runs\s*$/)).toBeInTheDocument();
    // Total cost = 0.03 rendered with the "total" suffix in the summary span.
    expect(screen.getByText(/\$0\.0300\s+total/)).toBeInTheDocument();
  });

  it('renders the failure glyph for unsuccessful runs', async () => {
    mockAuditLog.mockResolvedValue([
      entry({ success: false, error: 'rate limited', source_id: 'fail' }),
    ]);
    render(<SyncAuditPanel />);

    const failGlyph = await screen.findByTitle('rate limited');
    expect(failGlyph).toHaveTextContent('✗');
  });

  it('renders the success glyph for successful runs', async () => {
    mockAuditLog.mockResolvedValue([entry({ success: true })]);
    render(<SyncAuditPanel />);

    const okGlyph = await screen.findByTitle('Success');
    expect(okGlyph).toHaveTextContent('✓');
  });

  it('maps a rebuild scope through its label', async () => {
    mockAuditLog.mockResolvedValue([
      entry({ source_kind: 'rebuild', scope: 'rebuild:gmail:x-at-y-dot-com', source_id: 'r' }),
    ]);
    render(<SyncAuditPanel />);

    expect(await screen.findByText('Rebuild · gmail:x-at-y-dot-com')).toBeInTheDocument();
  });

  it('survives a fetch failure by leaving the empty state', async () => {
    const consoleError = vi.spyOn(console, 'error').mockImplementation(() => {});
    mockAuditLog.mockRejectedValue(new Error('boom'));
    render(<SyncAuditPanel />);

    // After the rejected fetch, loading clears and the empty state shows.
    await waitFor(() => expect(screen.getByText('No sync runs recorded yet.')).toBeInTheDocument());
    expect(consoleError).toHaveBeenCalled();
    consoleError.mockRestore();
  });

  it('renders a full table with headers when entries exist', async () => {
    mockAuditLog.mockResolvedValue([entry()]);
    render(<SyncAuditPanel />);

    const table = await screen.findByRole('table');
    expect(within(table).getByText('When')).toBeInTheDocument();
    expect(within(table).getByText('Source')).toBeInTheDocument();
    expect(within(table).getByText('Cost')).toBeInTheDocument();
  });
});

describe('timeAgo', () => {
  // Resolve to the fallback English so the `{n}` interpolation is exercised.
  const t = (_key: string, fallback?: string) => fallback ?? _key;
  const isoAgo = (ms: number) => new Date(Date.now() - ms).toISOString();

  it('covers every relative-time bucket and substitutes the {n} placeholder', () => {
    expect(timeAgo(isoAgo(0), t)).toBe('just now');
    expect(timeAgo(isoAgo(5 * 60_000), t)).toBe('5m ago');
    expect(timeAgo(isoAgo(3 * 60 * 60_000), t)).toBe('3h ago');
    expect(timeAgo(isoAgo(2 * 24 * 60 * 60_000), t)).toBe('2d ago');
  });
});
