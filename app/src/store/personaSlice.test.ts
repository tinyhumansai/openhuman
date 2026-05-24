import { REHYDRATE } from 'redux-persist';
import { describe, expect, it } from 'vitest';

import reducer, {
  MAX_PERSONA_DESCRIPTION_LEN,
  MAX_PERSONA_DISPLAY_NAME_LEN,
  resetPersona,
  selectPersonaDescription,
  selectPersonaDisplayName,
  setPersonaDescription,
  setPersonaDisplayName,
} from './personaSlice';
import { resetUserScopedState } from './resetActions';

describe('personaSlice', () => {
  it('starts empty', () => {
    const state = reducer(undefined, { type: '@@INIT' });
    expect(state.displayName).toBe('');
    expect(state.description).toBe('');
  });

  it('setPersonaDisplayName trims and stores the value', () => {
    const state = reducer(undefined, setPersonaDisplayName('  Nova  '));
    expect(state.displayName).toBe('Nova');
  });

  it('setPersonaDescription trims and stores the value', () => {
    const state = reducer(undefined, setPersonaDescription('  Calm and concise.  '));
    expect(state.description).toBe('Calm and concise.');
  });

  it('rejects an over-length display name, leaving the prior value intact', () => {
    const seeded = reducer(undefined, setPersonaDisplayName('Nova'));
    const tooLong = 'x'.repeat(MAX_PERSONA_DISPLAY_NAME_LEN + 1);
    const after = reducer(seeded, setPersonaDisplayName(tooLong));
    expect(after.displayName).toBe('Nova');
  });

  it('rejects an over-length description, leaving the prior value intact', () => {
    const seeded = reducer(undefined, setPersonaDescription('original'));
    const tooLong = 'x'.repeat(MAX_PERSONA_DESCRIPTION_LEN + 1);
    const after = reducer(seeded, setPersonaDescription(tooLong));
    expect(after.description).toBe('original');
  });

  it('accepts a display name exactly at the length limit', () => {
    const atLimit = 'x'.repeat(MAX_PERSONA_DISPLAY_NAME_LEN);
    const state = reducer(undefined, setPersonaDisplayName(atLimit));
    expect(state.displayName).toBe(atLimit);
  });

  it('allows clearing a field with an empty string', () => {
    const seeded = reducer(undefined, setPersonaDisplayName('Nova'));
    const cleared = reducer(seeded, setPersonaDisplayName('   '));
    expect(cleared.displayName).toBe('');
  });

  it('resetPersona clears both fields', () => {
    let state = reducer(undefined, setPersonaDisplayName('Nova'));
    state = reducer(state, setPersonaDescription('Calm.'));
    const after = reducer(state, resetPersona());
    expect(after.displayName).toBe('');
    expect(after.description).toBe('');
  });

  it('resets to initial state on resetUserScopedState', () => {
    let state = reducer(undefined, setPersonaDisplayName('Nova'));
    state = reducer(state, setPersonaDescription('Calm.'));
    const after = reducer(state, resetUserScopedState());
    expect(after.displayName).toBe('');
    expect(after.description).toBe('');
  });

  describe('REHYDRATE', () => {
    it('restores valid persisted fields', () => {
      const state = reducer(undefined, {
        type: REHYDRATE,
        key: 'persona',
        payload: { displayName: ' Nova ', description: ' Calm. ' },
      });
      expect(state.displayName).toBe('Nova');
      expect(state.description).toBe('Calm.');
    });

    it('scrubs over-length or non-string persisted fields to empty', () => {
      const state = reducer(undefined, {
        type: REHYDRATE,
        key: 'persona',
        payload: { displayName: 'x'.repeat(MAX_PERSONA_DISPLAY_NAME_LEN + 1), description: 42 },
      });
      expect(state.displayName).toBe('');
      expect(state.description).toBe('');
    });

    it('ignores rehydrate for other slices', () => {
      const seeded = reducer(undefined, setPersonaDisplayName('Nova'));
      const after = reducer(seeded, {
        type: REHYDRATE,
        key: 'mascot',
        payload: { displayName: 'overwritten' },
      });
      expect(after.displayName).toBe('Nova');
    });
  });

  describe('selectors', () => {
    it('read the current fields', () => {
      let persona = reducer(undefined, setPersonaDisplayName('Nova'));
      persona = reducer(persona, setPersonaDescription('Calm.'));
      expect(selectPersonaDisplayName({ persona })).toBe('Nova');
      expect(selectPersonaDescription({ persona })).toBe('Calm.');
    });
  });
});
