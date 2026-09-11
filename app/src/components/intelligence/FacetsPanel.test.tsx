import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import FacetsPanel from './FacetsPanel';

const listFacets = vi.fn();
const pinFacet = vi.fn();
const unpinFacet = vi.fn();
const forgetFacet = vi.fn();
const rebuildCache = vi.fn();
const getSettings = vi.fn();
const updateSettings = vi.fn();

vi.mock('../../services/api/learningApi', () => ({
  learningApi: {
    listFacets: (...args: unknown[]) => listFacets(...args),
    pinFacet: (...args: unknown[]) => pinFacet(...args),
    unpinFacet: (...args: unknown[]) => unpinFacet(...args),
    forgetFacet: (...args: unknown[]) => forgetFacet(...args),
    rebuildCache: (...args: unknown[]) => rebuildCache(...args),
    getSettings: (...args: unknown[]) => getSettings(...args),
    updateSettings: (...args: unknown[]) => updateSettings(...args),
  },
  splitFacetKey: (fullKey: string) => {
    const i = fullKey.indexOf('/');
    return i > 0
      ? { class: fullKey.slice(0, i), key: fullKey.slice(i + 1) }
      : { class: 'other', key: fullKey };
  },
}));

vi.mock('../../lib/i18n/I18nContext', () => ({
  useT: () => ({ t: (key: string, fallback?: string) => fallback ?? key }),
}));

describe('<FacetsPanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    getSettings.mockResolvedValue({ enabled: false });
    listFacets.mockResolvedValue([
      {
        key: 'style/verbosity',
        value: 'terse',
        state: 'active',
        user_state: 'auto',
        stability: 1.8,
        class: 'style',
      },
    ]);
    pinFacet.mockResolvedValue(undefined);
    unpinFacet.mockResolvedValue(undefined);
    forgetFacet.mockResolvedValue(undefined);
    rebuildCache.mockResolvedValue(undefined);
    updateSettings.mockResolvedValue({ enabled: true });
  });

  it('lists facets grouped by class', async () => {
    render(<FacetsPanel />);
    expect(await screen.findByTestId('facets-panel')).toBeInTheDocument();
    expect(screen.getByTestId('facets-class-style')).toBeInTheDocument();
    expect(screen.getByText('verbosity')).toBeInTheDocument();
    expect(screen.getByText('terse')).toBeInTheDocument();
  });

  it('pins a facet then refreshes the list', async () => {
    listFacets
      .mockResolvedValueOnce([
        {
          key: 'style/verbosity',
          value: 'terse',
          state: 'active',
          user_state: 'auto',
          stability: 1.8,
          class: 'style',
        },
      ])
      .mockResolvedValueOnce([
        {
          key: 'style/verbosity',
          value: 'terse',
          state: 'active',
          user_state: 'pinned',
          stability: 1.8,
          class: 'style',
        },
      ]);

    render(<FacetsPanel />);
    await screen.findByTestId('facet-pin-style/verbosity');
    fireEvent.click(screen.getByTestId('facet-pin-style/verbosity'));
    await waitFor(() => expect(pinFacet).toHaveBeenCalledWith('style/verbosity'));
    await waitFor(() => expect(listFacets).toHaveBeenCalledTimes(2));
  });

  it('toggles learning.enabled', async () => {
    render(<FacetsPanel />);
    const toggle = await screen.findByTestId('learning-enabled-toggle');
    expect(toggle).not.toBeChecked();
    fireEvent.click(toggle);
    await waitFor(() => expect(updateSettings).toHaveBeenCalledWith(true));
  });

  it('shows empty state when there are no facets', async () => {
    listFacets.mockResolvedValueOnce([]);
    render(<FacetsPanel />);
    expect(await screen.findByTestId('facets-empty')).toBeInTheDocument();
  });

  it('unpins and forgets facets, and rebuilds the cache', async () => {
    const pinned = {
      key: 'identity/name',
      value: 'Alice',
      state: 'active',
      user_state: 'pinned',
      stability: 2,
      class: 'identity',
    };
    listFacets.mockResolvedValue([pinned]);
    render(<FacetsPanel />);
    await screen.findByTestId('facet-row-identity/name');
    fireEvent.click(screen.getByTestId('facet-pin-identity/name'));
    await waitFor(() => expect(unpinFacet).toHaveBeenCalledWith('identity/name'));
    fireEvent.click(screen.getByTestId('facet-forget-identity/name'));
    await waitFor(() => expect(forgetFacet).toHaveBeenCalledWith('identity/name'));
    fireEvent.click(screen.getByTestId('facets-rebuild'));
    await waitFor(() => expect(rebuildCache).toHaveBeenCalled());
  });

  it('shows action errors and load errors', async () => {
    listFacets.mockRejectedValueOnce(new Error('load failed'));
    const firstView = render(<FacetsPanel />);
    expect(await screen.findByTestId('facets-panel-error')).toHaveTextContent('load failed');
    firstView.unmount();
    listFacets.mockResolvedValue([{ key: 'style/x', value: 'y', state: 'active', stability: 1 }]);
    pinFacet.mockRejectedValueOnce(new Error('pin failed'));
    render(<FacetsPanel />);
    await screen.findByTestId('facet-pin-style/x');
    fireEvent.click(screen.getByTestId('facet-pin-style/x'));
    expect(await screen.findByRole('alert')).toHaveTextContent('pin failed');
  });

  it('handles refresh and rebuild failures without losing the panel', async () => {
    listFacets.mockResolvedValue([
      { key: 'other/value', value: 'x', state: 'active', stability: 1 },
    ]);
    render(<FacetsPanel />);
    await screen.findByTestId('facet-row-other/value');

    listFacets.mockRejectedValueOnce(new Error('refresh failed'));
    fireEvent.click(screen.getByTestId('facet-pin-other/value'));
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('refresh failed'));

    rebuildCache.mockRejectedValueOnce(new Error('rebuild failed'));
    fireEvent.click(screen.getByTestId('facets-rebuild'));
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('rebuild failed'));
  });

  it('shows fallback action errors and renders unknown classes', async () => {
    listFacets.mockResolvedValueOnce([
      { key: 'unclassified', value: 'x', state: 'active', stability: 1 },
    ]);
    render(<FacetsPanel />);
    await screen.findByTestId('facets-class-other');

    forgetFacet.mockRejectedValueOnce('forget failed');
    fireEvent.click(screen.getByTestId('facet-forget-unclassified'));
    await waitFor(() =>
      expect(screen.getByRole('alert')).toHaveTextContent('brain.profile.actionError')
    );
  });

  it('reports a learning toggle failure', async () => {
    updateSettings.mockRejectedValueOnce(new Error('toggle failed'));
    render(<FacetsPanel />);
    const toggle = await screen.findByTestId('learning-enabled-toggle');
    fireEvent.click(toggle);
    await waitFor(() => expect(screen.getByRole('alert')).toHaveTextContent('toggle failed'));
  });

  it('reports a rebuild refresh failure', async () => {
    render(<FacetsPanel />);
    await screen.findByTestId('facets-panel');
    listFacets.mockRejectedValueOnce(new Error('rebuild refresh failed'));
    fireEvent.click(screen.getByTestId('facets-rebuild'));
    await waitFor(() =>
      expect(screen.getByRole('alert')).toHaveTextContent('rebuild refresh failed')
    );
  });
});
