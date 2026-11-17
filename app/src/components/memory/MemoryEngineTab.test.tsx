import { fireEvent, screen, waitFor, within } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { EngineState } from '../../services/api/memoryApi';
import { renderWithProviders } from '../../test/test-utils';
import { createLocalSessionToken } from '../../utils/localSession';
import MemoryEngineTab, {
  CORTEXDB_SELF_HOST_DOCS_URL,
  isLoopbackEndpoint,
} from './MemoryEngineTab';

const hoisted = vi.hoisted(() => ({
  engineSet: vi.fn(),
  openUrl: vi.fn(),
  signedIn: true,
  token: 'header.payload.sig',
  plan: null as string | null,
  toastAdd: vi.fn(),
}));

vi.mock('../ui/Toast', () => ({ toast: { add: (...a: unknown[]) => hoisted.toastAdd(...a) } }));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryEngineSet: (...a: unknown[]) => hoisted.engineSet(...a),
}));

vi.mock('./MemoryCortexAnnouncement', () => ({
  default: () => <div data-testid="stub-announcement" />,
}));
vi.mock('../../utils/openUrl', () => ({ openUrl: (...a: unknown[]) => hoisted.openUrl(...a) }));

vi.mock('../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({
    snapshot: {
      auth: { isAuthenticated: hoisted.signedIn, userId: hoisted.signedIn ? 'u1' : null },
      sessionToken: hoisted.signedIn ? hoisted.token : null,
      currentUser: hoisted.plan ? { subscription: { plan: hoisted.plan } } : null,
    },
  }),
}));

const OFF: EngineState = { engine: null, has_key: false, status: 'off', fetch_modes: [] };
const BUILTIN_ON: EngineState = {
  engine: 'tinyhumans',
  endpoint: 'https://api.tinyhumans.ai',
  has_key: true,
  status: 'ok',
  fetch_modes: ['hybrid'],
};
const CLOUD_ON: EngineState = {
  engine: 'cortexdb',
  endpoint: 'https://api-v1.cortexdb.ai',
  has_key: true,
  status: 'ok',
  fetch_modes: ['hybrid'],
};
const LOCAL_ON: EngineState = { ...CLOUD_ON, endpoint: 'http://localhost:3141' };

function renderTab(state: EngineState | null = OFF) {
  const onStateChange = vi.fn();
  renderWithProviders(<MemoryEngineTab state={state} onStateChange={onStateChange} />);
  return { onStateChange };
}

/** Pick a connection chip and return its panel. */
const pick = (option: string) => {
  const chip = screen.getByTestId(`memory-engine-${option}`);
  // Radix tabs activate on mousedown.
  fireEvent.mouseDown(chip, { button: 0 });
  fireEvent.click(chip);
  return screen.getByTestId(`memory-engine-panel-${option}`);
};
const type = (testId: string, value: string) =>
  fireEvent.change(screen.getByTestId(testId), { target: { value } });

beforeEach(() => {
  hoisted.engineSet.mockReset();
  hoisted.openUrl.mockReset().mockResolvedValue(undefined);
  hoisted.signedIn = true;
  hoisted.token = 'header.payload.sig';
  hoisted.plan = null;
  hoisted.toastAdd.mockReset();
});

describe('isLoopbackEndpoint', () => {
  it.each([
    ['http://localhost:3141', true],
    ['http://127.0.0.1:3141/', true],
    ['http://127.1.2.3:3141', true],
    ['http://[::1]:3141', true],
    ['https://localhost', true],
    [' http://localhost:3141 ', true],
    ['http://192.168.1.10:3141', false],
    // The URL parser rejects out-of-range octets before the host check runs.
    ['http://127.999.1.1:3141', false],
    ['http://127.256.0.1:3141', false],
    ['http://memory.example.internal:3141/', false],
    ['https://api-v1.cortexdb.ai', false],
    ['http://localhost.evil.com', false],
    ['ftp://localhost', false],
    ['localhost:3141', false],
    ['', false],
  ])('%s → %s', (endpoint, expected) => {
    expect(isLoopbackEndpoint(endpoint)).toBe(expected);
  });
});

describe('MemoryEngineTab', () => {
  it('is one CortexDB card that defaults to TinyHumans, with a plain prompt when off', () => {
    renderTab({ ...OFF, reason: 'legacy memory backend is unsupported' });
    const card = screen.getByTestId('memory-engines');
    expect(within(card).getByText('CortexDB')).toBeInTheDocument();
    for (const option of ['builtin', 'apikey', 'selfhost']) {
      expect(within(card).getByTestId(`memory-engine-${option}`)).toBeInTheDocument();
    }
    expect(within(card).getByTestId('memory-engine-builtin')).toHaveAttribute(
      'aria-selected',
      'true'
    );
    expect(within(card).getByText('Free')).toBeInTheDocument();
    expect(screen.getByTestId('memory-engine-panel-builtin')).toBeInTheDocument();
    // Nothing configured: no status badge, no chip marked in use.
    expect(screen.queryByTestId('memory-engine-status')).not.toBeInTheDocument();
    expect(screen.queryByTestId(/^memory-engine-chip-active-/)).not.toBeInTheDocument();
    const banner = screen.getByTestId('memory-engine-status-off');
    expect(banner).toHaveTextContent('Pick a provider below to start remembering.');
    // The core's developer-facing reason is not shown to people.
    expect(banner).not.toHaveTextContent('legacy memory backend');
  });

  it('opens on the connection in use', () => {
    renderTab(LOCAL_ON);
    expect(screen.getByTestId('memory-engine-selfhost')).toHaveAttribute('aria-selected', 'true');
    expect(screen.getByTestId('memory-engine-chip-active-selfhost')).toBeInTheDocument();
    expect(screen.getByTestId('memory-engine-status')).toHaveTextContent('In use');
  });

  it('lists upcoming engines as coming soon, with nothing to click', () => {
    renderTab();
    const soon = screen.getByTestId('memory-engines-soon');
    for (const id of ['supermemory', 'mem0', 'cognee', 'zep', 'letta']) {
      const card = within(soon).getByTestId(`memory-engine-soon-${id}`);
      expect(card).toHaveAttribute('aria-disabled', 'true');
      expect(within(card).getByText('Soon')).toBeInTheDocument();
      expect(within(card).queryByRole('button')).not.toBeInTheDocument();
    }
  });

  it('shows the CortexDB announcement on Memory → Provider', () => {
    renderTab(BUILTIN_ON);
    expect(screen.getByTestId('stub-announcement')).toBeInTheDocument();
  });

  it('leaves the CortexDB announcement out when embedded in onboarding', () => {
    renderWithProviders(<MemoryEngineTab state={OFF} onStateChange={vi.fn()} embedded />);
    expect(screen.queryByTestId('stub-announcement')).not.toBeInTheDocument();
  });

  it('leaves upcoming engines out when embedded in onboarding', () => {
    renderWithProviders(<MemoryEngineTab state={OFF} onStateChange={vi.fn()} embedded />);
    expect(screen.getByTestId('memory-engines')).toBeInTheDocument();
    expect(screen.queryByTestId('memory-engines-soon')).not.toBeInTheDocument();
  });

  describe('via TinyHumans', () => {
    it('is used in one click when signed in, and toasts the switch', async () => {
      hoisted.engineSet.mockResolvedValue(BUILTIN_ON);
      const { onStateChange } = renderTab();
      fireEvent.click(screen.getByTestId('memory-engine-builtin-submit'));
      await waitFor(() => expect(onStateChange).toHaveBeenCalledWith(BUILTIN_ON));
      expect(hoisted.engineSet).toHaveBeenCalledWith({ engine: 'tinyhumans' });
      expect(hoisted.toastAdd).toHaveBeenCalledWith(
        expect.objectContaining({
          type: 'success',
          title: 'Memory provider switched',
          description: 'CortexDB via TinyHumans now stores your memory.',
        })
      );
    });

    it('shows Connecting while the switch is in flight', () => {
      hoisted.engineSet.mockReturnValue(new Promise(() => undefined));
      renderTab();
      fireEvent.click(screen.getByTestId('memory-engine-builtin-submit'));
      expect(screen.getByTestId('memory-engine-builtin-submit')).toHaveTextContent('Connecting…');
      expect(screen.getByTestId('memory-engine-builtin-submit')).toBeDisabled();
    });

    it('is marked in use with nothing to press, and no endpoint line', () => {
      renderTab(BUILTIN_ON);
      expect(screen.getByTestId('memory-engine-status')).toHaveTextContent('In use');
      expect(screen.getByTestId('memory-engine-chip-active-builtin')).toBeInTheDocument();
      expect(screen.queryByTestId('memory-engine-builtin-submit')).not.toBeInTheDocument();
      expect(screen.getByTestId('memory-engine-panel-builtin')).not.toHaveTextContent(
        'https://api.tinyhumans.ai'
      );
    });

    it('cannot be selected when signed out, and says to sign in', () => {
      hoisted.signedIn = false;
      renderTab();
      expect(screen.getByTestId('memory-engine-builtin-sign-in')).toBeInTheDocument();
      expect(screen.getByTestId('memory-engine-builtin-submit')).toBeDisabled();
    });

    it('treats a local session token as signed out', () => {
      hoisted.token = createLocalSessionToken();
      renderTab();
      expect(screen.getByTestId('memory-engine-builtin-submit')).toBeDisabled();
    });

    it('shows Off on the configured engine while signed out, and offers it again', () => {
      hoisted.signedIn = false;
      renderTab({ ...OFF, engine: 'tinyhumans' });
      expect(screen.getByTestId('memory-engine-status')).toHaveTextContent('Off');
      expect(screen.getByTestId('memory-engine-builtin-submit')).toBeDisabled();
    });

    it('keeps a failed switch inside the panel', async () => {
      hoisted.engineSet.mockRejectedValue(new Error('backend unavailable'));
      renderTab();
      fireEvent.click(screen.getByTestId('memory-engine-builtin-submit'));
      expect(await screen.findByTestId('memory-engine-builtin-error')).toHaveTextContent(
        'backend unavailable'
      );
      expect(hoisted.toastAdd).not.toHaveBeenCalled();
    });

    it('says the plan’s memory is free in one line, with the details behind the info icon', () => {
      hoisted.plan = 'PRO';
      renderTab(BUILTIN_ON);
      expect(screen.getByTestId('memory-engine-builtin-note')).toHaveTextContent(
        'Free memory inference on your Pro plan, hosted by TinyHumans.'
      );
      // Quota, inference pricing and fair use sit behind an info icon.
      expect(screen.queryByTestId('memory-engine-fair-use')).not.toBeInTheDocument();
      fireEvent.click(screen.getByTestId('memory-engine-fair-use-trigger'));
      const terms = screen.getByTestId('memory-engine-fair-use');
      expect(screen.getByTestId('memory-engine-quota')).toHaveTextContent(
        'Your Pro plan includes 20 GB of memory storage.'
      );
      expect(terms).toHaveTextContent('Memory inference is never charged.');
      expect(terms).toHaveTextContent('Fair use');
      expect(terms).toHaveTextContent('No automated bulk uploads');
      fireEvent.click(within(terms).getByTestId('memory-engine-terms'));
      expect(hoisted.openUrl).toHaveBeenCalledWith('https://tinyhumans.ai/terms');
    });

    it.each([['FREE'], ['BASIC'], ['PRO'], [null]])('keeps Use enabled on plan %s', plan => {
      hoisted.plan = plan;
      renderTab(OFF);
      const panel = pick('builtin');
      expect(within(panel).getByTestId('memory-engine-builtin-submit')).toBeEnabled();
    });

    it('gives the Basic plan 1 GB of memory', () => {
      hoisted.plan = 'BASIC';
      renderTab(BUILTIN_ON);
      expect(screen.getByTestId('memory-engine-builtin-note')).toHaveTextContent(
        'Free memory inference on your Basic plan, hosted by TinyHumans.'
      );
      fireEvent.click(screen.getByTestId('memory-engine-fair-use-trigger'));
      expect(screen.getByTestId('memory-engine-quota')).toHaveTextContent(
        'Your Basic plan includes 1 GB of memory storage.'
      );
    });

    it('points free plans at Basic and Pro, with both quotas behind the info icon', () => {
      hoisted.plan = 'FREE';
      renderTab();
      expect(screen.getByTestId('memory-engine-builtin-note')).toHaveTextContent(
        'Free memory inference on Basic and Pro, hosted by TinyHumans.'
      );
      fireEvent.click(screen.getByTestId('memory-engine-fair-use-trigger'));
      expect(screen.getByTestId('memory-engine-quota')).toHaveTextContent(
        'Basic includes 1 GB of memory storage and Pro includes 20 GB.'
      );
    });
  });

  describe('your API key', () => {
    it('connects with only a key, clearing any custom endpoint', async () => {
      hoisted.engineSet.mockResolvedValue(CLOUD_ON);
      const { onStateChange } = renderTab();
      const panel = pick('apikey');
      const submit = within(panel).getByTestId('memory-engine-apikey-submit');
      expect(submit).toBeDisabled();
      type('memory-engine-apikey-key', 'ck_live_123');
      fireEvent.click(submit);
      await waitFor(() => expect(onStateChange).toHaveBeenCalledWith(CLOUD_ON));
      expect(hoisted.engineSet).toHaveBeenCalledWith({
        engine: 'cortexdb',
        endpoint: '',
        api_key: 'ck_live_123',
      });
      expect(hoisted.toastAdd).toHaveBeenCalledWith(
        expect.objectContaining({
          description: 'CortexDB with your own key now stores your memory.',
        })
      );
    });

    it('saves without a new key when already in use', async () => {
      hoisted.engineSet.mockResolvedValue(CLOUD_ON);
      renderTab(CLOUD_ON);
      expect(screen.getByTestId('memory-engine-apikey')).toHaveAttribute('aria-selected', 'true');
      // The card shows one line; Edit opens the form.
      expect(screen.getByTestId('memory-engine-connected-apikey')).toHaveTextContent(
        'Connected with your CortexDB API key.'
      );
      expect(screen.queryByTestId('memory-engine-apikey-key')).not.toBeInTheDocument();
      fireEvent.click(screen.getByTestId('memory-engine-apikey-edit'));
      expect(screen.getByTestId('memory-engine-apikey-dialog')).toBeInTheDocument();
      expect(screen.getByText(/A key is already saved/)).toBeInTheDocument();
      const submit = screen.getByTestId('memory-engine-apikey-submit');
      expect(submit).toHaveTextContent('Save');
      fireEvent.click(submit);
      await waitFor(() =>
        expect(hoisted.engineSet).toHaveBeenCalledWith({ engine: 'cortexdb', endpoint: '' })
      );
      expect(hoisted.toastAdd).toHaveBeenCalledWith({
        type: 'success',
        title: 'Memory settings saved',
      });
      await waitFor(() =>
        expect(screen.queryByTestId('memory-engine-apikey-dialog')).not.toBeInTheDocument()
      );
    });

    it('shows a rejected key inside the panel', async () => {
      hoisted.engineSet.mockRejectedValue(new Error('invalid api key'));
      renderTab();
      const panel = pick('apikey');
      type('memory-engine-apikey-key', 'bad');
      fireEvent.click(within(panel).getByTestId('memory-engine-apikey-submit'));
      expect(await screen.findByTestId('memory-engine-apikey-error')).toHaveTextContent(
        'invalid api key'
      );
    });

    it('reports a degraded or unreachable engine on its badge and banner', () => {
      const { unmount } = renderWithProviders(
        <MemoryEngineTab
          state={{ ...CLOUD_ON, status: 'degraded', reason: 'slow' }}
          onStateChange={vi.fn()}
        />
      );
      expect(screen.getByTestId('memory-engine-status')).toHaveTextContent('Degraded');
      expect(screen.getByTestId('memory-engine-status-degraded')).toHaveTextContent('slow');
      unmount();
      renderTab({ ...CLOUD_ON, status: 'down', reason: 'connection refused' });
      expect(screen.getByTestId('memory-engine-status')).toHaveTextContent('Unreachable');
      expect(screen.getByTestId('memory-engine-status-down')).toHaveTextContent(
        'connection refused'
      );
    });
  });

  describe('Local', () => {
    it('connects a loopback server with its key', async () => {
      hoisted.engineSet.mockResolvedValue(LOCAL_ON);
      const { onStateChange } = renderTab();
      const panel = pick('selfhost');
      type('memory-engine-selfhost-endpoint', 'http://localhost:3141');
      type('memory-engine-selfhost-key', 'local-key');
      fireEvent.click(within(panel).getByTestId('memory-engine-selfhost-submit'));
      await waitFor(() => expect(onStateChange).toHaveBeenCalledWith(LOCAL_ON));
      expect(hoisted.engineSet).toHaveBeenCalledWith({
        engine: 'cortexdb',
        endpoint: 'http://localhost:3141',
        api_key: 'local-key',
      });
    });

    it('refuses an endpoint that is not on this computer', () => {
      renderTab();
      const panel = pick('selfhost');
      type('memory-engine-selfhost-endpoint', 'http://192.168.1.10:3141');
      type('memory-engine-selfhost-key', 'k');
      expect(
        within(panel).getByTestId('memory-engine-selfhost-endpoint-error')
      ).toBeInTheDocument();
      expect(within(panel).getByTestId('memory-engine-selfhost-submit')).toBeDisabled();
    });

    it('fills in its endpoint when in use', () => {
      renderTab(LOCAL_ON);
      expect(screen.getByTestId('memory-engine-connected-selfhost')).toHaveTextContent(
        'Connected to CortexDB at http://localhost:3141.'
      );
      fireEvent.click(screen.getByTestId('memory-engine-selfhost-edit'));
      expect(
        (screen.getByTestId('memory-engine-selfhost-endpoint') as HTMLInputElement).value
      ).toBe('http://localhost:3141');
    });

    it('fills in the local endpoint when the state arrives after the first render', () => {
      const onStateChange = vi.fn();
      const { rerender } = renderWithProviders(
        <MemoryEngineTab state={OFF} onStateChange={onStateChange} />
      );
      rerender(<MemoryEngineTab state={LOCAL_ON} onStateChange={onStateChange} />);
      fireEvent.click(screen.getByTestId('memory-engine-selfhost-edit'));
      expect(
        (screen.getByTestId('memory-engine-selfhost-endpoint') as HTMLInputElement).value
      ).toBe('http://localhost:3141');
    });

    it('opens in a modal and is selected only once it connects', async () => {
      hoisted.engineSet.mockResolvedValue(LOCAL_ON);
      renderTab();
      pick('selfhost');
      expect(screen.getByTestId('memory-engine-selfhost-dialog')).toBeInTheDocument();
      expect(screen.getByTestId('memory-engine-selfhost')).toHaveAttribute(
        'aria-selected',
        'false'
      );
      type('memory-engine-selfhost-endpoint', 'http://localhost:3141');
      type('memory-engine-selfhost-key', 'k');
      fireEvent.click(screen.getByTestId('memory-engine-selfhost-submit'));
      await waitFor(() =>
        expect(screen.queryByTestId('memory-engine-selfhost-dialog')).not.toBeInTheDocument()
      );
    });

    it('keeps the previous chip when its modal is closed', () => {
      renderTab();
      pick('selfhost');
      fireEvent.keyDown(screen.getByTestId('memory-engine-selfhost-dialog'), { key: 'Escape' });
      expect(screen.queryByTestId('memory-engine-selfhost-dialog')).not.toBeInTheDocument();
      expect(screen.getByTestId('memory-engine-builtin')).toHaveAttribute('aria-selected', 'true');
    });

    it('links out to the CortexDB self-hosting guide', () => {
      renderTab();
      const panel = pick('selfhost');
      fireEvent.click(within(panel).getByTestId('memory-engine-selfhost-docs'));
      expect(hoisted.openUrl).toHaveBeenCalledWith(CORTEXDB_SELF_HOST_DOCS_URL);
    });
  });

  describe('Disabled', () => {
    const DISABLED: EngineState = {
      engine: 'none',
      has_key: false,
      status: 'off',
      fetch_modes: [],
    };

    it('turns memory off completely with one click', async () => {
      hoisted.engineSet.mockResolvedValue(DISABLED);
      const { onStateChange } = renderTab(BUILTIN_ON);
      fireEvent.click(screen.getByTestId('memory-engine-disable'));
      await waitFor(() => expect(onStateChange).toHaveBeenCalledWith(DISABLED));
      expect(hoisted.engineSet).toHaveBeenCalledWith({ engine: 'none' });
      expect(hoisted.toastAdd).toHaveBeenCalledWith(
        expect.objectContaining({ type: 'success', title: 'Memory disabled' })
      );
    });

    it('marks the Disabled card in use and explains how to turn memory back on', () => {
      renderTab(DISABLED);
      expect(screen.getByTestId('memory-engine-disabled-active')).toBeInTheDocument();
      expect(screen.queryByTestId('memory-engine-disable')).not.toBeInTheDocument();
      expect(screen.getByTestId('memory-engine-status-disabled')).toBeInTheDocument();
      expect(screen.queryByTestId('memory-engine-status-off')).not.toBeInTheDocument();
      expect(screen.queryByTestId('memory-engine-status')).not.toBeInTheDocument();
    });

    it('reports a failed disable', async () => {
      hoisted.engineSet.mockRejectedValue(new Error('boom'));
      const { onStateChange } = renderTab(BUILTIN_ON);
      fireEvent.click(screen.getByTestId('memory-engine-disable'));
      await waitFor(() =>
        expect(hoisted.toastAdd).toHaveBeenCalledWith(
          expect.objectContaining({ type: 'error', title: "Couldn't disable memory" })
        )
      );
      expect(onStateChange).not.toHaveBeenCalled();
    });

    it('is left out when embedded in onboarding', () => {
      renderWithProviders(<MemoryEngineTab state={OFF} onStateChange={vi.fn()} embedded />);
      expect(screen.queryByTestId('memory-engine-disabled')).not.toBeInTheDocument();
    });
  });

  it('shows a loading state until the engine state arrives', () => {
    renderTab(null);
    expect(screen.queryByTestId('memory-engine-tab')).not.toBeInTheDocument();
  });
});
