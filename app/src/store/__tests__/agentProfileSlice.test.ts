import { describe, expect, it } from 'vitest';

import reducer, { setAgentProfilesFromResponse } from '../agentProfileSlice';
import { resetUserScopedState } from '../resetActions';

describe('agentProfileSlice', () => {
  it('stores profiles and active profile from backend response', () => {
    const state = reducer(
      undefined,
      setAgentProfilesFromResponse({
        activeProfileId: 'planner',
        profiles: [
          {
            id: 'default',
            name: 'Orchestrator',
            description: 'Default',
            agentId: 'orchestrator',
            builtIn: true,
          },
          {
            id: 'planner',
            name: 'Planner',
            description: 'Planning profile',
            agentId: 'planner',
            builtIn: true,
          },
        ],
      })
    );

    expect(state.activeProfileId).toBe('planner');
    expect(state.profiles).toHaveLength(2);
    expect(state.error).toBeNull();
  });

  it('resets with other user-scoped state', () => {
    const dirty = reducer(
      undefined,
      setAgentProfilesFromResponse({
        activeProfileId: 'planner',
        profiles: [
          {
            id: 'planner',
            name: 'Planner',
            description: 'Planning profile',
            agentId: 'planner',
            builtIn: true,
          },
        ],
      })
    );

    const reset = reducer(dirty, resetUserScopedState());

    expect(reset.activeProfileId).toBe('default');
    expect(reset.profiles).toEqual([]);
    expect(reset.status).toBe('idle');
  });
});
