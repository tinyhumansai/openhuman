import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../../test/test-utils';
import BrowserConnectionsPanel from './BrowserConnectionsPanel';

const mocks = vi.hoisted(() => ({ getConfig: vi.fn(), update: vi.fn(), rpc: vi.fn() }));
vi.mock('../../../lib/i18n/I18nContext', () => ({ useT: () => ({ t: (key: string) => key }) }));
vi.mock('../../../utils/tauriCommands/config', () => ({
  openhumanGetConfig: mocks.getConfig,
  openhumanUpdateBrowserSettings: mocks.update,
}));
vi.mock('../../../services/coreRpcClient', () => ({ callCoreRpc: mocks.rpc }));

beforeEach(() => {
  vi.clearAllMocks();
  mocks.getConfig.mockResolvedValue({
    result: {
      browser_billing_route: 'direct_openrouter',
      config: {
        browser: {
          enabled: false,
          headless: true,
          viewport_width: 1280,
          viewport_height: 720,
          profile_mode: 'fresh',
          max_task_steps: 20,
          task_timeout_secs: 120,
        },
        http_request: { allowed_domains: ['selenium.dev'] },
      },
    },
  });
  mocks.rpc.mockResolvedValue({
    result: { modules: [{ id: 'tinycomputer', state: 'available' }] },
  });
  mocks.update.mockResolvedValue({ result: { config: {} } });
});

describe('BrowserConnectionsPanel', () => {
  it('shows Chrome readiness and the shared website policy', async () => {
    renderWithProviders(<BrowserConnectionsPanel />);
    expect(screen.getByText('connections.earlyAlphaNotice')).toBeInTheDocument();
    expect(await screen.findByText('selenium.dev')).toBeInTheDocument();
    expect(screen.getByText('connections.browser.notVerified')).toBeInTheDocument();
    // Module state lives on the Computer page's status card now.
    expect(mocks.rpc).not.toHaveBeenCalled();
  });

  it('leaves the alpha notice to the Computer page when embedded', async () => {
    renderWithProviders(<BrowserConnectionsPanel embedded />);
    expect(await screen.findByText('selenium.dev')).toBeInTheDocument();
    expect(screen.queryByText('connections.earlyAlphaNotice')).not.toBeInTheDocument();
  });

  it('saves browser settings and checks browser readiness through core RPC', async () => {
    renderWithProviders(<BrowserConnectionsPanel />);
    await screen.findByText('selenium.dev');
    fireEvent.click(screen.getByLabelText('connections.browser.enabled'));
    fireEvent.click(screen.getByText('connections.browser.save'));
    await waitFor(() =>
      expect(mocks.update).toHaveBeenCalledWith(
        expect.objectContaining({ enabled: true, profile_mode: 'fresh' })
      )
    );
    await waitFor(() => expect(screen.getByText('connections.browser.testBrowser')).toBeEnabled());
    mocks.rpc.mockResolvedValueOnce({ module_ready: true, chrome_ready: true });
    fireEvent.click(screen.getByText('connections.browser.testBrowser'));
    await waitFor(() =>
      expect(mocks.rpc).toHaveBeenCalledWith({
        method: 'openhuman.modules_browser_check_readiness',
      })
    );
  });

  it('links to allowed websites through the Connections router', async () => {
    renderWithProviders(<BrowserConnectionsPanel />);
    await screen.findByText('selenium.dev');
    expect(
      screen.getByRole('link', { name: 'connections.browser.manageWebsites' })
    ).toHaveAttribute('href', '/connections?tab=search');
  });

  it.each([
    ['connections.browser.width', '319'],
    ['connections.browser.width', '3841'],
    ['connections.browser.height', '239'],
    ['connections.browser.height', '2161'],
    ['connections.browser.maxSteps', '0'],
    ['connections.browser.maxSteps', '101'],
    ['connections.browser.timeout', '4'],
    ['connections.browser.timeout', '601'],
    ['connections.browser.width', '320.5'],
  ])('rejects an out-of-bounds %s value %s before calling the core', async (label, value) => {
    renderWithProviders(<BrowserConnectionsPanel />);
    await screen.findByText('selenium.dev');
    fireEvent.change(screen.getByLabelText(label), { target: { value } });
    fireEvent.click(screen.getByText('connections.browser.save'));
    expect(await screen.findByText('connections.browser.boundsRequired')).toBeInTheDocument();
    expect(mocks.update).not.toHaveBeenCalled();
  });

  it.each([
    ['320', '240', '1', '5'],
    ['3840', '2160', '100', '600'],
  ])('accepts inclusive boundaries', async (width, height, steps, timeout) => {
    renderWithProviders(<BrowserConnectionsPanel />);
    await screen.findByText('selenium.dev');
    fireEvent.change(screen.getByLabelText('connections.browser.width'), {
      target: { value: width },
    });
    fireEvent.change(screen.getByLabelText('connections.browser.height'), {
      target: { value: height },
    });
    fireEvent.change(screen.getByLabelText('connections.browser.maxSteps'), {
      target: { value: steps },
    });
    fireEvent.change(screen.getByLabelText('connections.browser.timeout'), {
      target: { value: timeout },
    });
    fireEvent.click(screen.getByText('connections.browser.save'));
    await waitFor(() =>
      expect(mocks.update).toHaveBeenCalledWith(
        expect.objectContaining({
          viewport_width: Number(width),
          viewport_height: Number(height),
          max_task_steps: Number(steps),
          task_timeout_secs: Number(timeout),
        })
      )
    );
  });

  it('sends only the accepted fields when the config carries legacy browser keys', async () => {
    mocks.getConfig.mockResolvedValue({
      result: {
        config: {
          browser: {
            enabled: false,
            headless: true,
            viewport_width: 1280,
            viewport_height: 800,
            profile_mode: 'fresh',
            max_task_steps: 20,
            task_timeout_secs: 120,
            chrome_path: null,
            allowed_domains: [],
            session_name: null,
            backend: 'auto',
          },
          http_request: { allowed_domains: ['selenium.dev'] },
        },
      },
    });
    renderWithProviders(<BrowserConnectionsPanel />);
    await screen.findByText('selenium.dev');
    fireEvent.click(screen.getByLabelText('connections.browser.enabled'));
    fireEvent.click(screen.getByText('connections.browser.save'));
    await waitFor(() => expect(mocks.update).toHaveBeenCalledTimes(1));
    const sent = mocks.update.mock.calls[0][0] as Record<string, unknown>;
    expect(Object.keys(sent).sort()).toEqual(
      [
        'chrome_path',
        'download_dir',
        'enabled',
        'headless',
        'learn_from_tasks',
        'max_task_steps',
        'profile_mode',
        'profile_path',
        'task_timeout_secs',
        'viewport_height',
        'viewport_width',
      ].sort()
    );
    expect(sent).toMatchObject({ enabled: true, viewport_height: 800 });
  });

  it('saves learning from finished tasks off when switched off', async () => {
    renderWithProviders(<BrowserConnectionsPanel />);
    await screen.findByText('selenium.dev');
    const learn = screen.getByLabelText('connections.browser.learnFromTasks');
    expect(learn).toBeChecked();
    fireEvent.click(learn);
    fireEvent.click(screen.getByText('connections.browser.save'));
    await waitFor(() =>
      expect(mocks.update).toHaveBeenCalledWith(
        expect.objectContaining({ learn_from_tasks: false })
      )
    );
  });

  it('forgets what finished tasks learned through core RPC', async () => {
    renderWithProviders(<BrowserConnectionsPanel />);
    await screen.findByText('selenium.dev');
    mocks.rpc.mockResolvedValueOnce({ forgotten: 3 });
    fireEvent.click(screen.getByText('connections.browser.forgetSites'));
    await waitFor(() =>
      expect(mocks.rpc).toHaveBeenCalledWith({
        method: 'openhuman.modules_browser_forget_sites',
        params: {},
      })
    );
    expect(await screen.findByRole('status')).toHaveTextContent(
      'connections.browser.sitesForgotten'
    );
    mocks.rpc.mockRejectedValueOnce(new Error('could not forget'));
    fireEvent.click(screen.getByText('connections.browser.forgetSites'));
    await waitFor(() => expect(screen.getByRole('status')).toHaveTextContent('could not forget'));
  });

  it('checks Chrome by opening and closing a core browser session', async () => {
    renderWithProviders(<BrowserConnectionsPanel />);
    await screen.findByText('selenium.dev');
    mocks.rpc.mockResolvedValueOnce({ module_ready: true, chrome_ready: true, error: null });
    fireEvent.click(screen.getByText('connections.browser.testBrowser'));
    await waitFor(() =>
      expect(mocks.rpc).toHaveBeenCalledWith({
        method: 'openhuman.modules_browser_check_readiness',
      })
    );
    expect(await screen.findByText('connections.browser.chromeReady')).toBeInTheDocument();
  });

  it('shows an unsuccessful Chrome probe without claiming readiness', async () => {
    renderWithProviders(<BrowserConnectionsPanel />);
    await screen.findByText('selenium.dev');
    mocks.rpc.mockResolvedValueOnce({
      module_ready: true,
      chrome_ready: false,
      error: 'Chrome unavailable',
    });
    fireEvent.click(screen.getByText('connections.browser.testBrowser'));
    expect(await screen.findByText('connections.browser.chromeNotReady')).toBeInTheDocument();
    expect(screen.getByRole('status')).toHaveTextContent('Chrome unavailable');
  });
});
