import { configureStore } from '@reduxjs/toolkit';
import { render, screen, waitFor } from '@testing-library/react';
import { Provider } from 'react-redux';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';

import mascotReducer, {
  setCustomMascotGifUrl,
  setSelectedMascotId,
} from '../../../../store/mascotSlice';
import type { MascotManifest, MascotManifestEntry } from './types';
import { useMascotManifest } from './useMascotManifest';

const fetchMascotManifest = vi.hoisted(() => vi.fn());
// Fully mock the service (matching BackendRiveMascot.test) so no real module
// instance lingers. findMascot/defaultMascot keep their real semantics.
vi.mock('./manifestService', () => ({
  fetchMascotManifest,
  findMascot: (m: MascotManifest, id: string | null | undefined) =>
    id ? m.mascots.find(x => x.id === id) : undefined,
  defaultMascot: (m: MascotManifest) => m.mascots.find(x => x.status === 'ready') ?? m.mascots[0],
}));

function entry(id: string, status: 'ready' | 'draft'): MascotManifestEntry {
  return {
    id,
    name: id,
    description: '',
    status,
    tags: [],
    stateEngine: {
      idlePoseCycle: ['idle'],
      states: { idle: 'idle', thinking: 'thinking' },
      visemeCodes: ['sil'],
    },
    files: [
      { path: `${id}.riv`, bytes: 1, role: 'runtime', sha256: id, url: `https://x/${id}.riv` },
    ],
  };
}

const MANIFEST: MascotManifest = {
  schemaVersion: 1,
  generatedAt: '',
  mascots: [entry('toshi', 'draft'), entry('tiny-mascot', 'ready')],
  source: { repository: '', branch: '', commit: '' },
};

// A render-based probe — surfaces the hook's output into the DOM. We render the
// hook through a real component (not renderHook) so a rejecting manifest fetch
// is consumed exactly like it is in production, with no orphaned promise.
function Probe() {
  const { entry: e, loading, error } = useMascotManifest();
  return (
    <div>
      <span data-testid="entry">{e?.id ?? 'none'}</span>
      <span data-testid="loading">{String(loading)}</span>
      <span data-testid="error">{error?.message ?? ''}</span>
    </div>
  );
}

function renderProbe(selectedId: string | null, customGifUrl: string | null = null) {
  const store = configureStore({ reducer: { mascot: mascotReducer } });
  if (selectedId) store.dispatch(setSelectedMascotId(selectedId));
  if (customGifUrl) store.dispatch(setCustomMascotGifUrl(customGifUrl));
  const view = render(
    <Provider store={store}>
      <Probe />
    </Provider>
  );
  return { store, ...view };
}

beforeEach(() => fetchMascotManifest.mockReset());
afterEach(() => vi.restoreAllMocks());

describe('useMascotManifest', () => {
  it('resolves the selected mascot when set', async () => {
    fetchMascotManifest.mockResolvedValue(MANIFEST);
    renderProbe('toshi');
    await waitFor(() => expect(screen.getByTestId('entry')).toHaveTextContent('toshi'));
    expect(screen.getByTestId('loading')).toHaveTextContent('false');
  });

  it('falls back to the default (first ready) mascot when none selected', async () => {
    fetchMascotManifest.mockResolvedValue(MANIFEST);
    const { store } = renderProbe(null);
    await waitFor(() => expect(screen.getByTestId('entry')).toHaveTextContent('tiny-mascot'));
    expect(store.getState().mascot.selectedMascotId).toBeNull();
  });

  it('reconciles a stale selected mascot id after the manifest loads', async () => {
    fetchMascotManifest.mockResolvedValue(MANIFEST);
    const { store } = renderProbe('removed-mascot');
    await waitFor(() => expect(screen.getByTestId('entry')).toHaveTextContent('tiny-mascot'));
    await waitFor(() => expect(store.getState().mascot.selectedMascotId).toBe('tiny-mascot'));
  });

  it('does not overwrite a custom GIF selection with the manifest fallback', async () => {
    fetchMascotManifest.mockResolvedValue(MANIFEST);
    const { store } = renderProbe(null, 'https://example.com/custom.gif');

    await waitFor(() => expect(screen.getByTestId('entry')).toHaveTextContent('tiny-mascot'));

    expect(store.getState().mascot.customMascotGifUrl).toBe('https://example.com/custom.gif');
    expect(store.getState().mascot.selectedMascotId).toBeNull();
  });

  // The fetch-failure path is covered end-to-end in manifestService.test.ts
  // ("rejects when the network fails and there is no snapshot") and the
  // entry:null fallback render is covered in HumanPage.test.tsx, so the hook's
  // catch branch is exercised without re-triggering a vitest-v4 quirk where
  // awaiting into a settling (but handled) rejection surfaces as a test error.
});
