import { screen } from '@testing-library/react';
import { beforeEach, describe, expect, it, vi } from 'vitest';

import { renderWithProviders } from '../../test/test-utils';
import MemoryEngineSetup from './MemoryEngineSetup';

const hoisted = vi.hoisted(() => ({ engineGet: vi.fn(), tabProps: vi.fn() }));

vi.mock('../../services/api/memoryApi', async importOriginal => ({
  ...(await importOriginal<typeof import('../../services/api/memoryApi')>()),
  memoryEngineGet: (...a: unknown[]) => hoisted.engineGet(...a),
}));

vi.mock('./MemoryEngineTab', () => ({
  default: (props: { state: { engine: string | null; status: string } | null }) => {
    hoisted.tabProps(props);
    return (
      <div data-testid="engine-tab">
        {props.state ? `${props.state.engine ?? 'none'}:${props.state.status}` : 'loading'}
      </div>
    );
  },
}));

beforeEach(() => {
  hoisted.engineGet.mockReset();
  hoisted.tabProps.mockReset();
});

describe('MemoryEngineSetup', () => {
  it('feeds the engine picker the current engine state', async () => {
    hoisted.engineGet.mockResolvedValue({
      engine: 'tinyhumans',
      has_key: false,
      status: 'ok',
      fetch_modes: [],
    });
    renderWithProviders(<MemoryEngineSetup />);
    expect(await screen.findByText('tinyhumans:ok')).toBeInTheDocument();
    expect(hoisted.tabProps).toHaveBeenLastCalledWith(expect.objectContaining({ embedded: true }));
  });

  it('falls back to off when the engine cannot be read', async () => {
    hoisted.engineGet.mockRejectedValue(new Error('offline'));
    renderWithProviders(<MemoryEngineSetup />);
    expect(await screen.findByText('none:off')).toBeInTheDocument();
  });
});
