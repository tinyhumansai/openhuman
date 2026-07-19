import { beforeEach, describe, expect, it, vi } from 'vitest';

import { hydrateEmergencyState } from './hydrateEmergencyState';

// Mock emergencyApi before importing the module under test
const emergencyStatusMock = vi.fn();
vi.mock('../api/emergencyApi', () => ({ emergencyStatus: () => emergencyStatusMock() }));

// Mock hydrateHalt action creator
const hydrateHaltMock = vi.fn((x: unknown) => ({ type: 'safety/hydrateHalt', payload: x }));
vi.mock('../../store/safetySlice', () => ({ hydrateHalt: (x: unknown) => hydrateHaltMock(x) }));

describe('hydrateEmergencyState', () => {
  const dispatch = vi.fn();

  beforeEach(() => {
    dispatch.mockClear();
    emergencyStatusMock.mockReset();
    hydrateHaltMock.mockClear();
  });

  it('dispatches hydrateHalt with the result of emergencyStatus on success', async () => {
    const status = { engaged: true, reason: 'cli', source: 'cli', engaged_at_ms: 12345 };
    emergencyStatusMock.mockResolvedValue(status);

    await hydrateEmergencyState(dispatch);

    expect(hydrateHaltMock).toHaveBeenCalledWith(status);
    expect(dispatch).toHaveBeenCalledWith({ type: 'safety/hydrateHalt', payload: status });
  });

  it('dispatches hydrateHalt when halt is not engaged', async () => {
    const status = { engaged: false };
    emergencyStatusMock.mockResolvedValue(status);

    await hydrateEmergencyState(dispatch);

    expect(hydrateHaltMock).toHaveBeenCalledWith(status);
    expect(dispatch).toHaveBeenCalledTimes(1);
  });

  it('swallows errors from emergencyStatus and does not dispatch', async () => {
    emergencyStatusMock.mockRejectedValue(new Error('core unavailable'));

    // Must not throw
    await expect(hydrateEmergencyState(dispatch)).resolves.toBeUndefined();
    expect(dispatch).not.toHaveBeenCalled();
  });
});
