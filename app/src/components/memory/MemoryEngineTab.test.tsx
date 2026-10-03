import { fireEvent, screen, waitFor } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import type { EngineDescriptor, EngineState } from '../../services/api/memoryApi';
import { renderWithProviders } from '../../test/test-utils';
import MemoryEngineTab from './MemoryEngineTab';

const hoisted = vi.hoisted(() => ({ enginesList: vi.fn(), engineSet: vi.fn(), signedIn: true }));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryEnginesList: (...a: unknown[]) => hoisted.enginesList(...a),
  memoryEngineSet: (...a: unknown[]) => hoisted.engineSet(...a),
}));

vi.mock('../../providers/CoreStateProvider', () => ({
  useCoreState: () => ({
    snapshot: {
      auth: { isAuthenticated: hoisted.signedIn, userId: hoisted.signedIn ? 'u1' : null },
      sessionToken: hoisted.signedIn ? 'header.payload.sig' : null,
    },
  }),
}));

vi.mock('../../utils/localSession', () => ({ isLocalSessionToken: () => false }));

const TINYHUMANS: EngineDescriptor = {
  id: 'tinyhumans',
  label: 'TinyHumans',
  description: 'Hosted memory',
  hosted: true,
  needs_endpoint: false,
  needs_key: false,
  default_endpoint: null,
  fetch_modes: ['hybrid'],
};
const CORTEXDB: EngineDescriptor = {
  id: 'cortexdb',
  label: 'CortexDB',
  description: 'Your own CortexDB',
  hosted: false,
  // As the engine reports it: the endpoint has a default, so it is optional.
  needs_endpoint: false,
  needs_key: true,
  default_endpoint: 'https://api-v1.cortexdb.ai',
  fetch_modes: ['keyword', 'vector', 'hybrid'],
};

const OFF: EngineState = { engine: null, has_key: false, status: 'off', fetch_modes: [] };

function renderTab(state: EngineState | null = OFF) {
  const onStateChange = vi.fn();
  renderWithProviders(<MemoryEngineTab state={state} onStateChange={onStateChange} />);
  return { onStateChange };
}

beforeEach(() => {
  hoisted.enginesList
    .mockReset()
    .mockResolvedValue({ engines: [TINYHUMANS, CORTEXDB], active: null });
  hoisted.engineSet.mockReset();
  hoisted.signedIn = true;
});

describe('MemoryEngineTab', () => {
  it('lists the engines and explains that memory is off', async () => {
    renderTab();
    expect(await screen.findByTestId('memory-engine-tinyhumans')).toBeInTheDocument();
    expect(screen.getByTestId('memory-engine-cortexdb')).toBeInTheDocument();
    expect(screen.getByTestId('memory-engine-status-off')).toHaveTextContent('Memory is off');
  });

  it('shows the engine-reported reason when memory is off', async () => {
    renderTab({ ...OFF, reason: 'Signed out and no CortexDB key' });
    expect(await screen.findByTestId('memory-engine-status-off')).toHaveTextContent(
      'Signed out and no CortexDB key'
    );
  });

  it('selects TinyHumans in one click when signed in', async () => {
    const next: EngineState = {
      engine: 'tinyhumans',
      has_key: false,
      status: 'ok',
      fetch_modes: ['hybrid'],
    };
    hoisted.engineSet.mockResolvedValue(next);
    const { onStateChange } = renderTab();
    fireEvent.click(await screen.findByTestId('memory-engine-tinyhumans-use'));
    await waitFor(() => expect(hoisted.engineSet).toHaveBeenCalledWith({ engine: 'tinyhumans' }));
    expect(onStateChange).toHaveBeenCalledWith(next);
  });

  it('disables TinyHumans with an explanation when signed out', async () => {
    hoisted.signedIn = false;
    renderTab();
    const use = await screen.findByTestId('memory-engine-tinyhumans-use');
    expect(use).toBeDisabled();
    expect(screen.getByTestId('memory-engine-tinyhumans-detail')).toHaveTextContent(
      'Sign in required'
    );
  });

  it('connects CortexDB with the default endpoint and a key', async () => {
    const next: EngineState = {
      engine: 'cortexdb',
      endpoint: 'https://api-v1.cortexdb.ai',
      has_key: true,
      status: 'ok',
      fetch_modes: ['keyword', 'vector', 'hybrid'],
    };
    hoisted.engineSet.mockResolvedValue(next);
    const { onStateChange } = renderTab();
    fireEvent.click(await screen.findByTestId('memory-engine-cortexdb-use'));

    const endpoint = screen.getByTestId('memory-engine-connect-cortexdb-endpoint');
    expect(endpoint).toHaveValue('https://api-v1.cortexdb.ai');
    const submit = screen.getByTestId('memory-engine-connect-cortexdb-submit');
    expect(submit).toBeDisabled(); // key required
    fireEvent.change(screen.getByTestId('memory-engine-connect-cortexdb-key'), {
      target: { value: 'secret' },
    });
    fireEvent.click(submit);

    await waitFor(() =>
      expect(hoisted.engineSet).toHaveBeenCalledWith({
        engine: 'cortexdb',
        endpoint: 'https://api-v1.cortexdb.ai',
        api_key: 'secret',
      })
    );
    expect(onStateChange).toHaveBeenCalledWith(next);
    await waitFor(() =>
      expect(screen.queryByTestId('memory-engine-connect-cortexdb')).not.toBeInTheDocument()
    );
  });

  it('connects CortexDB to a local server instead of the default endpoint', async () => {
    hoisted.engineSet.mockResolvedValue({
      engine: 'cortexdb',
      endpoint: 'http://127.0.0.1:3141',
      has_key: true,
      status: 'ok',
      fetch_modes: ['hybrid'],
    });
    renderTab();
    fireEvent.click(await screen.findByTestId('memory-engine-cortexdb-use'));

    fireEvent.change(screen.getByTestId('memory-engine-connect-cortexdb-endpoint'), {
      target: { value: ' http://127.0.0.1:3141 ' },
    });
    fireEvent.change(screen.getByTestId('memory-engine-connect-cortexdb-key'), {
      target: { value: 'local-key' },
    });
    fireEvent.click(screen.getByTestId('memory-engine-connect-cortexdb-submit'));

    await waitFor(() =>
      expect(hoisted.engineSet).toHaveBeenCalledWith({
        engine: 'cortexdb',
        endpoint: 'http://127.0.0.1:3141',
        api_key: 'local-key',
      })
    );
  });

  it('refuses to connect CortexDB with a blank endpoint', async () => {
    renderTab();
    fireEvent.click(await screen.findByTestId('memory-engine-cortexdb-use'));
    fireEvent.change(screen.getByTestId('memory-engine-connect-cortexdb-endpoint'), {
      target: { value: '   ' },
    });
    fireEvent.change(screen.getByTestId('memory-engine-connect-cortexdb-key'), {
      target: { value: 'secret' },
    });
    expect(screen.getByTestId('memory-engine-connect-cortexdb-submit')).toBeDisabled();
  });

  it('marks the active engine and reports a degraded status', async () => {
    renderTab({
      engine: 'cortexdb',
      endpoint: 'https://cortex.local',
      has_key: true,
      status: 'degraded',
      reason: 'slow answers',
      fetch_modes: ['hybrid'],
    });
    expect(await screen.findByTestId('memory-engine-cortexdb-active')).toHaveTextContent('Active');
    expect(screen.getByTestId('memory-engine-cortexdb-detail')).toHaveTextContent(
      'https://cortex.local'
    );
    expect(screen.getByTestId('memory-engine-status-degraded')).toHaveTextContent('slow answers');
  });

  it('shows a failed switch without closing the picker', async () => {
    hoisted.engineSet.mockRejectedValue(new Error('UNAUTHORIZED: bad key'));
    renderTab();
    fireEvent.click(await screen.findByTestId('memory-engine-tinyhumans-use'));
    expect(await screen.findByTestId('memory-engine-save-error')).toHaveTextContent('bad key');
  });

  it('shows an error when the engine list cannot load', async () => {
    hoisted.enginesList.mockRejectedValue(new Error('core offline'));
    renderTab();
    expect(await screen.findByTestId('memory-engine-load-error')).toHaveTextContent('core offline');
  });
});
