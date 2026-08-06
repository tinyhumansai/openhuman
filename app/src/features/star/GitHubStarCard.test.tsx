import { screen, waitFor } from '@testing-library/react';
import userEvent from '@testing-library/user-event';
import { afterEach, beforeEach, describe, expect, test, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import { GitHubStarCard } from './GitHubStarCard';

const mocks = vi.hoisted(() => ({ openUrl: vi.fn(), trackAnalyticsEvent: vi.fn() }));

vi.mock('../../utils/openUrl', () => ({ openUrl: mocks.openUrl }));
vi.mock('../../components/analytics', () => ({ trackAnalyticsEvent: mocks.trackAnalyticsEvent }));

beforeEach(() => {
  mocks.openUrl.mockResolvedValue(undefined);
});

afterEach(() => {
  Object.values(mocks).forEach(m => m.mockReset());
});

describe('GitHubStarCard', () => {
  test('renders the CTA when not yet dismissed', () => {
    renderWithProviders(<GitHubStarCard />);
    expect(screen.getByTestId('github-star-cta')).toBeInTheDocument();
    expect(screen.getByText('Enjoying OpenHuman?')).toBeInTheDocument();
    expect(screen.getByText('Star on GitHub')).toBeInTheDocument();
  });

  test('does not render when already dismissed (persisted dismissal)', () => {
    renderWithProviders(<GitHubStarCard />, {
      preloadedState: { githubStar: { dismissed: true } },
    });
    expect(screen.queryByTestId('github-star-cta')).not.toBeInTheDocument();
  });

  test('star click opens the repo, tracks analytics, and dismisses durably', async () => {
    const user = userEvent.setup();
    const { store } = renderWithProviders(<GitHubStarCard />);

    await user.click(screen.getByText('Star on GitHub'));

    await waitFor(() => expect(mocks.openUrl).toHaveBeenCalledTimes(1));
    expect(mocks.openUrl.mock.calls[0][0]).toBe('https://github.com/tinyhumansai/openhuman');
    expect(mocks.trackAnalyticsEvent).toHaveBeenCalledWith('github_star_cta_clicked');
    // Acting on the CTA retires it: the flag is set and the card unmounts.
    expect(store.getState().githubStar.dismissed).toBe(true);
    await waitFor(() => expect(screen.queryByTestId('github-star-cta')).not.toBeInTheDocument());
  });

  test('star click still retires the CTA when openUrl rejects (failure path)', async () => {
    const user = userEvent.setup();
    mocks.openUrl.mockRejectedValueOnce(new Error('opener unavailable'));
    const { store } = renderWithProviders(<GitHubStarCard />);

    await user.click(screen.getByText('Star on GitHub'));

    // A failed browser hand-off must not throw and must still retire the CTA:
    // the durable dismissal is dispatched before the async openUrl resolves.
    await waitFor(() => expect(mocks.openUrl).toHaveBeenCalledTimes(1));
    expect(store.getState().githubStar.dismissed).toBe(true);
    await waitFor(() => expect(screen.queryByTestId('github-star-cta')).not.toBeInTheDocument());
  });

  test('dismiss click hides the CTA, tracks analytics, and never opens a URL', async () => {
    const user = userEvent.setup();
    const { store } = renderWithProviders(<GitHubStarCard />);

    await user.click(screen.getByText('Not now'));

    expect(mocks.trackAnalyticsEvent).toHaveBeenCalledWith('github_star_cta_dismissed');
    expect(mocks.openUrl).not.toHaveBeenCalled();
    expect(store.getState().githubStar.dismissed).toBe(true);
    await waitFor(() => expect(screen.queryByTestId('github-star-cta')).not.toBeInTheDocument());
  });
});
