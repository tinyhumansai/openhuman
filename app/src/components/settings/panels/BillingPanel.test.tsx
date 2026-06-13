import { fireEvent, render, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

const navigateBack = vi.fn();

vi.mock('../hooks/useSettingsNavigation', () => ({
  useSettingsNavigation: () => ({
    navigateBack,
    navigateToSettings: vi.fn(),
    navigateToTeamManagement: vi.fn(),
    breadcrumbs: [],
  }),
}));

const openUrlMock = vi.fn();
vi.mock('../../../utils/openUrl', () => ({ openUrl: (url: string) => openUrlMock(url) }));

async function importPanel() {
  vi.resetModules();
  const mod = await import('./BillingPanel');
  return mod.default;
}

describe('<BillingPanel />', () => {
  beforeEach(() => {
    vi.clearAllMocks();
    openUrlMock.mockResolvedValue(undefined);
  });

  it('renders the "billing moved to web" view without auto-opening the browser', async () => {
    const Panel = await importPanel();
    render(<Panel />);

    // Billing no longer auto-opens the dashboard on mount (auto-open removed):
    // the panel just explains billing moved to the web.
    expect(
      screen.getByText(
        /Subscription changes, payment methods, credits, and invoices are now managed/
      )
    ).toBeInTheDocument();
    expect(screen.getByRole('button', { name: 'Open billing dashboard' })).toBeInTheDocument();
    // No openUrl call happens on mount.
    expect(openUrlMock).not.toHaveBeenCalled();
  });

  it('opens the dashboard when the user clicks the primary button', async () => {
    const Panel = await importPanel();
    render(<Panel />);

    fireEvent.click(screen.getByRole('button', { name: 'Open billing dashboard' }));
    await waitFor(() => expect(openUrlMock).toHaveBeenCalledTimes(1));
    expect(openUrlMock).toHaveBeenLastCalledWith('https://tinyhumans.ai/dashboard');
  });

  it('invokes the navigation back handler from both the header and the inline button', async () => {
    const Panel = await importPanel();
    render(<Panel />);

    // The SettingsHeader back button (aria-label "Back") and the inline
    // "Back to settings" button both route through navigateBack.
    fireEvent.click(screen.getByRole('button', { name: 'Back' }));
    fireEvent.click(screen.getByRole('button', { name: 'Back to settings' }));
    expect(navigateBack).toHaveBeenCalledTimes(2);
  });
});
