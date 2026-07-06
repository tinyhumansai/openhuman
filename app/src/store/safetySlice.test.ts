import { describe, it, expect } from 'vitest';
import reducer, { setHalt, clearHalt, hydrateHalt } from './safetySlice';

describe('safetySlice', () => {
  it('starts not halted', () => {
    expect(reducer(undefined, { type: '@@init' })).toEqual({ halted: false });
  });
  it('setHalt marks halted with reason/source/since', () => {
    const s = reducer(undefined, setHalt({ reason: 'user', source: 'user', since: 42 }));
    expect(s).toEqual({ halted: true, reason: 'user', source: 'user', since: 42 });
  });
  it('clearHalt resets', () => {
    const halted = reducer(undefined, setHalt({ reason: 'x' }));
    expect(reducer(halted, clearHalt())).toEqual({ halted: false });
  });
  it('hydrateHalt maps a HaltState snapshot', () => {
    const s = reducer(undefined, hydrateHalt({ engaged: true, reason: 'boot', engaged_at_ms: 7, source: 'system' }));
    expect(s.halted).toBe(true);
    expect(s.reason).toBe('boot');
    expect(s.since).toBe(7);
  });
});
