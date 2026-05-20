import { describe, expect, it } from 'vitest';

import { selectBlockingState } from '../connectivitySelectors';
import type { ConnectivityState } from '../connectivitySlice';
import type { RootState } from '../index';

const make = (over: Partial<ConnectivityState>): RootState =>
  ({
    // The selector only reads `connectivity`. Cast through unknown so we don't
    // have to fabricate the rest of the root state.
    connectivity: {
      internet: 'online',
      core: 'reachable',
      backend: 'connected',
      lastError: {},
      ...over,
    },
  }) as unknown as RootState;

describe('selectBlockingState', () => {
  it('returns ok when all three channels are healthy', () => {
    expect(selectBlockingState(make({}))).toBe('ok');
  });

  it('prioritises internet outage over everything else', () => {
    expect(
      selectBlockingState(
        make({ internet: 'offline', core: 'unreachable', backend: 'disconnected' })
      )
    ).toBe('internet-offline');
  });

  it('returns core-unreachable when only the sidecar is down', () => {
    expect(selectBlockingState(make({ core: 'unreachable' }))).toBe('core-unreachable');
  });

  it('returns backend-only when just the websocket is degraded', () => {
    expect(selectBlockingState(make({ backend: 'disconnected' }))).toBe('backend-only');
    expect(selectBlockingState(make({ backend: 'connecting' }))).toBe('backend-only');
  });
});
