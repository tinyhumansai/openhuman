import { fireEvent, render, screen } from '@testing-library/react';
import { MemoryRouter, useNavigate } from 'react-router-dom';
import { describe, expect, it, vi } from 'vitest';

import { AnalyticsPageTracker } from './AnalyticsTracker';

const mocks = vi.hoisted(() => ({ trackPageView: vi.fn() }));

vi.mock('../../services/analytics', () => ({ trackPageView: mocks.trackPageView }));

describe('analytics tracking primitives', () => {
  it('tracks a page when its path changes', () => {
    function PageHarness() {
      const navigate = useNavigate();
      return (
        <>
          <AnalyticsPageTracker />
          <button type="button" onClick={() => navigate('/flows')}>
            Navigate
          </button>
        </>
      );
    }

    render(
      <MemoryRouter initialEntries={['/chat']}>
        <PageHarness />
      </MemoryRouter>
    );
    expect(mocks.trackPageView).toHaveBeenLastCalledWith('/chat');

    fireEvent.click(screen.getByRole('button', { name: 'Navigate' }));
    expect(mocks.trackPageView).toHaveBeenLastCalledWith('/flows');
  });
});
