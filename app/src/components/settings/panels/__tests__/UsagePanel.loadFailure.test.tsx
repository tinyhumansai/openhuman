/**
 * UsagePanel — the Background tab's snapshot-load failure, for a rejection
 * that is not an `Error`.
 *
 * The sibling spec covers a rejected `new Error('rpc down')`, which exercises
 * the true arm of `err instanceof Error ? err.message : String(err)`
 * (UsagePanel.tsx:87). The false arm was unexecuted — one of the three
 * branches keeping the panel at 78.57% despite 100% statement and line
 * coverage.
 *
 * It is worth an assertion because the failure is silent-ish: a bare string or
 * an object rejection is a normal shape for an RPC layer to produce, and
 * without the `String(err)` fallback the panel renders "undefined" where the
 * reason should be — which reads as a bug in the panel rather than a failure
 * of the call behind it.
 *
 * # What this file deliberately does NOT test
 *
 * The other two uncovered branches are the `if (!cancelled)` guards at :84 and
 * :87, which suppress a state write when the effect is torn down before the
 * promise settles. Under React 18 a setState on an unmounted component is a
 * silent no-op — no warning, no thrown error, no rendered difference — so
 * removing either guard changes nothing a test at this level can observe. Any
 * test I wrote for them would pass with the guard deleted, which is exactly
 * the vacuous shape this suite is meant to avoid, so they are left uncovered
 * and called out here instead.
 */
import { screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, test, vi } from 'vitest';

import { loadAISettings } from '../../../../services/api/aiSettingsApi';
import { renderWithProviders } from '../../../../test/test-utils';
import UsagePanel from '../UsagePanel';

vi.mock('../../../dashboard/CostDashboardPanel', () => ({
  default: () => <div data-testid="stub-cost-dashboard" />,
}));

vi.mock('../AIPanel', () => ({
  BackgroundLoopControls: () => <div data-testid="stub-background-loops" />,
}));

vi.mock('../TokenUsagePanel', () => ({ default: () => <div data-testid="stub-token-usage" /> }));

vi.mock('../../../../services/api/aiSettingsApi', async () => {
  const actual = await vi.importActual<typeof import('../../../../services/api/aiSettingsApi')>(
    '../../../../services/api/aiSettingsApi'
  );
  return { ...actual, loadAISettings: vi.fn() };
});

vi.mock('../../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack: vi.fn(),
    navigateToSettings: vi.fn(),
    breadcrumbs: [],
  }),
}));

const mockLoad = vi.mocked(loadAISettings);

beforeEach(() => {
  mockLoad.mockReset();
});

describe('UsagePanel background snapshot failures', () => {
  test('stringifies a bare string rejection instead of rendering undefined', async () => {
    mockLoad.mockRejectedValue('core offline');
    renderWithProviders(<UsagePanel />, { initialEntries: ['/settings/usage#background'] });

    await waitFor(() =>
      expect(screen.getByTestId('usage-background-tab')).toHaveTextContent(/core offline/)
    );
    expect(screen.getByTestId('usage-background-tab')).not.toHaveTextContent(/undefined/);
    // The controls must stay hidden — a failed snapshot means there is nothing
    // safe to edit, and rendering them would offer writes against unknown state.
    expect(screen.queryByTestId('stub-background-loops')).not.toBeInTheDocument();
  });

  test('stringifies a non-Error object rejection', async () => {
    // An RPC layer rejecting with a plain object is the other common shape.
    // `String({})` is "[object Object]" — not useful, but it is what the code
    // promises, and it is not "undefined". Pinning it stops a refactor from
    // silently dropping to `err.message` on a value that has none.
    mockLoad.mockRejectedValue({ code: 500 });
    renderWithProviders(<UsagePanel />, { initialEntries: ['/settings/usage#background'] });

    await waitFor(() =>
      expect(screen.getByTestId('usage-background-tab')).toHaveTextContent(/\[object Object\]/)
    );
    expect(screen.getByTestId('usage-background-tab')).not.toHaveTextContent(/undefined/);
  });
});
