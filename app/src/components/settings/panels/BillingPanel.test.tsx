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

vi.mock('../components/PageBackButton', () => ({
  default: ({ label, onClick }: { label: string; onClick: () => void }) => (
    <button type="button" data-testid="page-back-button" onClick={onClick}>
      {label}
    </button>
  ),
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

  it('opens the billing dashboard on mount and shows the post-open status', async () => {
    const Panel = await importPanel();
    render(<Panel />);

    // The auto-open effect fires once; while the promise is pending the
    // panel reports the "opening" status. Once it resolves we settle on
    // the "idle" copy that tells users the browser should be open.
    await waitFor(() => {
      expect(openUrlMock).toHaveBeenCalledWith('https://tinyhumans.ai/dashboard');
    });
    await waitFor(() => {
      expect(
        screen.getByText('If your browser did not open, use the button above.')
      ).toBeInTheDocument();
    });
  });

  it('surfaces an error message when openUrl rejects on mount', async () => {
    openUrlMock.mockRejectedValueOnce(new Error('boom'));
    const Panel = await importPanel();
    render(<Panel />);

    // First call is the auto-open attempt that rejects -> error copy renders.
    await waitFor(() => {
      expect(
        screen.getByText('The browser could not be opened automatically. Use the button above.')
      ).toBeInTheDocument();
    });
  });

  it('re-opens the dashboard when the user clicks the primary button', async () => {
    const Panel = await importPanel();
    render(<Panel />);

    // Wait for the mount effect call to complete first so the second call
    // is unambiguously the click.
    await waitFor(() => expect(openUrlMock).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByRole('button', { name: 'Open billing dashboard' }));
    await waitFor(() => expect(openUrlMock).toHaveBeenCalledTimes(2));
    expect(openUrlMock).toHaveBeenLastCalledWith('https://tinyhumans.ai/dashboard');
  });

  it('invokes the navigation back handler from both the header and the inline button', async () => {
    const Panel = await importPanel();
    render(<Panel />);
    await waitFor(() => expect(openUrlMock).toHaveBeenCalledTimes(1));

    fireEvent.click(screen.getByTestId('page-back-button'));
    fireEvent.click(screen.getByRole('button', { name: 'Back to settings' }));
    expect(navigateBack).toHaveBeenCalledTimes(2);
  });
});
