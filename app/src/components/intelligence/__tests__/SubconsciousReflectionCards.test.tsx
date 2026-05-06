/**
 * Vitest for SubconsciousReflectionCards (#623).
 *
 * Covers: empty state, card rendering for Observe + Notify, action
 * button visibility, dismiss optimistic hide, and the act → mark-acted
 * RPC wiring.
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
    disposition: 'notify',
    proposed_action: 'Pull mentions',
    source_refs: ['entity:phoenix'],
    created_at: 1,
    surfaced_at: null,
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
    renderWithProviders(
      <SubconsciousReflectionCards activeThreadId="thread-1" initialReflections={[]} />
    );
    expect(screen.getByTestId('reflection-cards-empty')).toBeTruthy();
  });

  it('renders Notify reflections with Act + Dismiss buttons', () => {
    renderWithProviders(
      <SubconsciousReflectionCards activeThreadId="thread-1" initialReflections={[refl()]} />
    );
    expect(screen.getByText('Phoenix surge')).toBeTruthy();
    expect(screen.getByText('Hotness spike')).toBeTruthy();
    expect(screen.getByText('In conversation')).toBeTruthy();
    expect(screen.getByTestId('reflection-act-r-1')).toBeTruthy();
    expect(screen.getByTestId('reflection-dismiss-r-1')).toBeTruthy();
  });

  it('renders Observe reflections without Act button', () => {
    renderWithProviders(
      <SubconsciousReflectionCards
        activeThreadId="thread-1"
        initialReflections={[refl({ id: 'obs-1', disposition: 'observe', proposed_action: null })]}
      />
    );
    expect(screen.getByText('Observed')).toBeTruthy();
    expect(screen.queryByTestId('reflection-act-obs-1')).toBeNull();
    expect(screen.getByTestId('reflection-dismiss-obs-1')).toBeTruthy();
  });

  it('hides card optimistically on dismiss tap', async () => {
    mockedDismissReflection.mockResolvedValueOnce({ result: { dismissed: 'r-1' }, logs: [] });
    renderWithProviders(
      <SubconsciousReflectionCards activeThreadId="thread-1" initialReflections={[refl()]} />
    );
    fireEvent.click(screen.getByTestId('reflection-dismiss-r-1'));
    await waitFor(() => {
      expect(screen.queryByTestId('reflection-card-r-1')).toBeNull();
    });
    expect(mockedDismissReflection).toHaveBeenCalledWith('r-1');
  });

  it('act fires actOnReflection RPC with target thread + hides card', async () => {
    mockedActOnReflection.mockResolvedValueOnce({
      result: { request_id: 'req-1', reflection_id: 'r-1' },
      logs: [],
    });
    renderWithProviders(
      <SubconsciousReflectionCards activeThreadId="thread-active" initialReflections={[refl()]} />
    );
    fireEvent.click(screen.getByTestId('reflection-act-r-1'));
    await waitFor(() => {
      expect(mockedActOnReflection).toHaveBeenCalledWith('r-1', 'thread-active');
    });
    await waitFor(() => {
      expect(screen.queryByTestId('reflection-card-r-1')).toBeNull();
    });
  });

  it('disables Act button when no active thread', () => {
    renderWithProviders(
      <SubconsciousReflectionCards activeThreadId={null} initialReflections={[refl()]} />
    );
    const btn = screen.getByTestId('reflection-act-r-1') as HTMLButtonElement;
    expect(btn.disabled).toBe(true);
  });

  it('hides reflections that already have dismissed_at or acted_on_at', () => {
    renderWithProviders(
      <SubconsciousReflectionCards
        activeThreadId="thread-1"
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
    renderWithProviders(<SubconsciousReflectionCards activeThreadId="thread-1" />);
    await waitFor(() => {
      expect(mockedListReflections).toHaveBeenCalled();
    });
    await waitFor(() => {
      expect(screen.getByTestId('reflection-card-fetched')).toBeTruthy();
    });
  });
});
