import { describe, expect, it } from 'vitest';

import reducer, {
  resetAIState,
  setAIError,
  setAIStatus,
  setCurrentSessionId,
  setLoadedSkillsCount,
  setMemoryInitialized,
  updateAIConfig,
} from '../aiSlice';

describe('aiSlice', () => {
  const initialState = reducer(undefined, { type: '@@INIT' });

  describe('initial state', () => {
    it('should have idle status', () => {
      expect(initialState.status).toBe('idle');
    });

    it('should have null error', () => {
      expect(initialState.error).toBeNull();
    });

    it('should have null session ID', () => {
      expect(initialState.currentSessionId).toBeNull();
    });

    it('should have 0 loaded skills', () => {
      expect(initialState.loadedSkillsCount).toBe(0);
    });

    it('should not be memory initialized', () => {
      expect(initialState.memoryInitialized).toBe(false);
    });

    it('should have default skills repo URL', () => {
      expect(initialState.config.skillsRepoUrl).toBe('openhuman/openhuman-skills');
    });
  });

  describe('setAIStatus', () => {
    it('should update status', () => {
      const state = reducer(initialState, setAIStatus('ready'));
      expect(state.status).toBe('ready');
    });

    it('should clear error when status is not error', () => {
      const errorState = reducer(initialState, setAIError('something broke'));
      expect(errorState.error).toBe('something broke');
      const readyState = reducer(errorState, setAIStatus('ready'));
      expect(readyState.error).toBeNull();
    });

    it('should keep error when status is error', () => {
      const state = reducer({ ...initialState, error: 'old error' }, setAIStatus('error'));
      expect(state.status).toBe('error');
      // error is not cleared since status is "error"
    });
  });

  describe('setAIError', () => {
    it('should set error message and status to error', () => {
      const state = reducer(initialState, setAIError('Init failed'));
      expect(state.status).toBe('error');
      expect(state.error).toBe('Init failed');
    });
  });

  describe('setCurrentSessionId', () => {
    it('should set session ID', () => {
      const state = reducer(initialState, setCurrentSessionId('session-abc'));
      expect(state.currentSessionId).toBe('session-abc');
    });

    it('should clear session ID with null', () => {
      const withSession = reducer(initialState, setCurrentSessionId('session-abc'));
      const cleared = reducer(withSession, setCurrentSessionId(null));
      expect(cleared.currentSessionId).toBeNull();
    });
  });

  describe('setLoadedSkillsCount', () => {
    it('should update skills count', () => {
      const state = reducer(initialState, setLoadedSkillsCount(5));
      expect(state.loadedSkillsCount).toBe(5);
    });
  });

  describe('setMemoryInitialized', () => {
    it('should update memory initialized flag', () => {
      const state = reducer(initialState, setMemoryInitialized(true));
      expect(state.memoryInitialized).toBe(true);
    });
  });

  describe('updateAIConfig', () => {
    it('should merge partial config', () => {
      const state = reducer(
        initialState,
        updateAIConfig({ llmEndpoint: 'http://localhost:8080', llmModel: 'custom-v1' })
      );
      expect(state.config.llmEndpoint).toBe('http://localhost:8080');
      expect(state.config.llmModel).toBe('custom-v1');
      expect(state.config.skillsRepoUrl).toBe('openhuman/openhuman-skills');
    });

    it('should override existing config values', () => {
      const state1 = reducer(initialState, updateAIConfig({ llmEndpoint: 'http://v1' }));
      const state2 = reducer(state1, updateAIConfig({ llmEndpoint: 'http://v2' }));
      expect(state2.config.llmEndpoint).toBe('http://v2');
    });
  });

  describe('resetAIState', () => {
    it('should reset to initial state', () => {
      const modified = reducer(initialState, setAIStatus('ready'));
      const modified2 = reducer(modified, setCurrentSessionId('session-x'));
      const reset = reducer(modified2, resetAIState());
      expect(reset.status).toBe('idle');
      expect(reset.currentSessionId).toBeNull();
      expect(reset.loadedSkillsCount).toBe(0);
    });
  });
});
