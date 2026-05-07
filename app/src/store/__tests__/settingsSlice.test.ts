import { describe, expect, it } from 'vitest';

import reducer, {
  disconnectChannelConnection,
  resetChannelConnectionsState,
  setChannelConnectionStatus,
} from '../channelConnectionsSlice';
import notificationReducer, { clearAll, setPreference } from '../notificationSlice';

describe('Settings Reducers', () => {
  describe('channelConnectionsSlice (Settings)', () => {
    it('sets channel connection status and error', () => {
      const state = reducer(
        undefined,
        setChannelConnectionStatus({
          channel: 'telegram',
          authMode: 'managed_dm',
          status: 'error',
          lastError: 'Auth failed',
        })
      );
      expect(state.connections.telegram.managed_dm?.status).toBe('error');
      expect(state.connections.telegram.managed_dm?.lastError).toBe('Auth failed');
    });

    it('disconnects a channel connection', () => {
      const state = reducer(
        undefined,
        disconnectChannelConnection({ channel: 'telegram', authMode: 'managed_dm' })
      );
      expect(state.connections.telegram.managed_dm?.status).toBe('disconnected');
      expect(state.connections.telegram.managed_dm?.lastError).toBeUndefined();
    });

    it('resets the entire channel connections state', () => {
      const initialState = reducer(undefined, { type: '@@INIT' });
      const modified = reducer(
        initialState,
        setChannelConnectionStatus({ channel: 'discord', authMode: 'oauth', status: 'connected' })
      );
      expect(modified).not.toEqual(initialState);

      const reset = reducer(modified, resetChannelConnectionsState());
      expect(reset).toEqual(initialState);
    });
  });

  describe('notificationSlice (Settings)', () => {
    it('updates notification category preference', () => {
      const initialState = notificationReducer(undefined, { type: '@@INIT' });
      expect(initialState.preferences.messages).toBe(true);

      const state = notificationReducer(
        initialState,
        setPreference({ category: 'messages', enabled: false })
      );
      expect(state.preferences.messages).toBe(false);
      expect(state.preferences.agents).toBe(true); // Should not affect other categories
    });

    it('clears all notifications', () => {
      const stateWithNotifications = {
        items: [
          {
            id: '1',
            category: 'system',
            title: 'Test',
            body: 'Test',
            timestamp: Date.now(),
            read: false,
          },
        ],
        preferences: { messages: true, agents: true, skills: true, system: true },
        integrationItems: [],
        integrationUnreadCount: 0,
        integrationLoading: false,
        integrationError: null,
      };

      // @ts-ignore - testing reducer directly with partial state
      const state = notificationReducer(stateWithNotifications, clearAll());
      expect(state.items).toEqual([]);
    });
  });
});
