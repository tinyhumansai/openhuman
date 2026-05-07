/**
 * Vitest for SubconsciousReflectionCards (#623).
 *
 * Covers: empty state, card rendering with/without proposed_action,
 * action button visibility, dismiss optimistic hide, the act → spawn-
 * thread RPC wiring, and the onNavigateToThread callback.
 */
import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import {
  actOnReflection,
  dismissReflection,
  listReflections,
  type Reflection,
} from '../../../utils/tauriCommands/subconscious';
import SubconsciousReflectionCards from '../SubconsciousReflectionCards';

// Mock just the subconscious tauriCommand surface — leaves the rest of
// the module untouched so the component's static imports don't blow up.
vi.mock('../../../utils/tauriCommands/subconscious', async () => {
  const actual = await vi.importActual<typeof import('../../../utils/tauriCommands/subconscious')>(
    '../../../utils/tauriCommands/subconscious'
  );
  return {
    ...actual,
    listReflections: vi.fn(),
    actOnReflection: vi.fn(),
    dismissReflection: vi.fn(),
  };
});

const mockedListReflections = vi.mocked(listReflections);
const mockedActOnReflection = vi.mocked(actOnReflection);
const mockedDismissReflection = vi.mocked(dismissReflection);

function refl(overrides: Partial<Reflection> = {}): Reflection {
  return {
    id: 'r-1',
    kind: 'hotness_spike',
    body: 'Phoenix surge',
    proposed_action: 'Pull mentions',
    source_refs: ['entity:phoenix'],
    created_at: 1,
    acted_on_at: null,
    dismissed_at: null,
    ...overrides,
  };
}

describe('SubconsciousReflectionCards', () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it('renders empty state when no reflections', () => {
    renderWithProviders(<SubconsciousReflectionCards initialReflections={[]} />);
    expect(screen.getByTestId('reflection-cards-empty')).toBeTruthy();
  });

  it('renders reflections with Act + Dismiss buttons when proposed_action is present', () => {
    renderWithProviders(<SubconsciousReflectionCards initialReflections={[refl()]} />);
    expect(screen.getByText('Phoenix surge')).toBeTruthy();
    expect(screen.getByText('Hotness spike')).toBeTruthy();
    expect(screen.getByTestId('reflection-act-r-1')).toBeTruthy();
    expect(screen.getByTestId('reflection-dismiss-r-1')).toBeTruthy();
  });

  it('renders reflections without Act button when proposed_action is null', () => {
    renderWithProviders(
      <SubconsciousReflectionCards
        initialReflections={[refl({ id: 'obs-1', proposed_action: null })]}
      />
    );
    expect(screen.queryByTestId('reflection-act-obs-1')).toBeNull();
    expect(screen.getByTestId('reflection-dismiss-obs-1')).toBeTruthy();
  });

  it('hides card optimistically on dismiss tap', async () => {
    mockedDismissReflection.mockResolvedValueOnce({ result: { dismissed: 'r-1' }, logs: [] });
    renderWithProviders(<SubconsciousReflectionCards initialReflections={[refl()]} />);
    fireEvent.click(screen.getByTestId('reflection-dismiss-r-1'));
    await waitFor(() => {
      expect(screen.queryByTestId('reflection-card-r-1')).toBeNull();
    });
    expect(mockedDismissReflection).toHaveBeenCalledWith('r-1');
  });

  it('act fires actOnReflection RPC, hides card, and calls onNavigateToThread with the new thread id', async () => {
    mockedActOnReflection.mockResolvedValueOnce({
      result: { reflection_id: 'r-1', thread_id: 'spawned-thread-1' },
      logs: [],
    });
    const onNavigateToThread = vi.fn();
    renderWithProviders(
      <SubconsciousReflectionCards
        initialReflections={[refl()]}
        onNavigateToThread={onNavigateToThread}
      />
    );
    fireEvent.click(screen.getByTestId('reflection-act-r-1'));
    await waitFor(() => {
      expect(mockedActOnReflection).toHaveBeenCalledWith('r-1');
    });
    await waitFor(() => {
      expect(screen.queryByTestId('reflection-card-r-1')).toBeNull();
    });
    expect(onNavigateToThread).toHaveBeenCalledWith('spawned-thread-1');
  });

  it('hides reflections that already have dismissed_at or acted_on_at', () => {
    renderWithProviders(
      <SubconsciousReflectionCards
        initialReflections={[
          refl({ id: 'visible' }),
          refl({ id: 'gone-acted', acted_on_at: 100 }),
          refl({ id: 'gone-dismissed', dismissed_at: 100 }),
        ]}
      />
    );
    expect(screen.getByTestId('reflection-card-visible')).toBeTruthy();
    expect(screen.queryByTestId('reflection-card-gone-acted')).toBeNull();
    expect(screen.queryByTestId('reflection-card-gone-dismissed')).toBeNull();
  });

  it('fetches reflections on mount via listReflections (when no initial seed)', async () => {
    mockedListReflections.mockResolvedValueOnce({ result: [refl({ id: 'fetched' })], logs: [] });
    renderWithProviders(<SubconsciousReflectionCards />);
    await waitFor(() => {
      expect(mockedListReflections).toHaveBeenCalled();
    });
    await waitFor(() => {
      expect(screen.getByTestId('reflection-card-fetched')).toBeTruthy();
    });
  });

  it('shows the error banner when listReflections rejects', async () => {
    mockedListReflections.mockRejectedValueOnce(new Error('boom: rpc unreachable'));
    renderWithProviders(<SubconsciousReflectionCards />);
    await waitFor(() => {
      expect(screen.getByTestId('reflection-cards-error')).toBeTruthy();
    });
    expect(screen.getByTestId('reflection-cards-error').textContent).toContain(
      'boom: rpc unreachable'
    );
  });

  it('rolls the optimistic dismiss back when dismissReflection rejects', async () => {
    mockedDismissReflection.mockRejectedValueOnce(new Error('rpc denied'));
    renderWithProviders(<SubconsciousReflectionCards initialReflections={[refl()]} />);
    fireEvent.click(screen.getByTestId('reflection-dismiss-r-1'));
    // First the card disappears (optimistic), then it comes back when the
    // rejection lands in the catch handler — the rollback path is what
    // bumps coverage on the otherwise-untested catch branch.
    await waitFor(() => {
      expect(screen.queryByTestId('reflection-card-r-1')).toBeNull();
    });
    await waitFor(() => {
      expect(screen.getByTestId('reflection-card-r-1')).toBeTruthy();
    });
  });

  it('surfaces the error banner when actOnReflection rejects', async () => {
    mockedActOnReflection.mockRejectedValueOnce(new Error('act failed'));
    const onNavigateToThread = vi.fn();
    renderWithProviders(
      <SubconsciousReflectionCards
        initialReflections={[refl()]}
        onNavigateToThread={onNavigateToThread}
      />
    );
    fireEvent.click(screen.getByTestId('reflection-act-r-1'));
    await waitFor(() => {
      expect(screen.getByTestId('reflection-cards-error')).toBeTruthy();
    });
    // Card stays visible (act failed → no optimistic hide finalises) and
    // the navigate callback is *not* fired.
    expect(screen.getByTestId('reflection-card-r-1')).toBeTruthy();
    expect(onNavigateToThread).not.toHaveBeenCalled();
  });
});
