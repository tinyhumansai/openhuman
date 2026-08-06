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
});
