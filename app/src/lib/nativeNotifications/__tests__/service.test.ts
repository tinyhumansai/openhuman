import { beforeEach, describe, expect, it, vi } from 'vitest';

import { store } from '../../../store';
import { setPreference } from '../../../store/notificationSlice';
import {
  __handleChatDoneForTests,
  __handleCoreNotificationForTests,
  __resetForTests,
} from '../service';
import { showNativeNotification } from '../tauriBridge';

vi.mock('../tauriBridge', () => ({ showNativeNotification: vi.fn() }));

vi.mock('../../../services/socketService', () => ({
  socketService: { on: vi.fn(), off: vi.fn() },
}));

describe('nativeNotifications service', () => {
  beforeEach(() => {
    __resetForTests();
    vi.clearAllMocks();
    // Clean slate for each test — clear any notifications persisted by prior ones.
    store.dispatch({ type: 'notifications/clearAll' });
    store.dispatch(setPreference({ category: 'agents', enabled: true }));
  });

  it('dispatches chat_done into the agents category of the center', () => {
    __handleChatDoneForTests({ thread_id: 't1', request_id: 'r1', full_response: 'Hello world' });
    const items = store.getState().notifications.items;
    expect(items).toHaveLength(1);
    expect(items[0].category).toBe('agents');
    expect(items[0].title).toBe('Agent reply ready');
    expect(items[0].body).toBe('Hello world');
    expect(items[0].deepLink).toBe('/chat');
  });

  it('truncates very long responses to 160 chars', () => {
    __handleChatDoneForTests({ thread_id: 't1', request_id: 'r1', full_response: 'a'.repeat(500) });
    const items = store.getState().notifications.items;
    expect(items[0].body.length).toBe(160);
    expect(items[0].body.endsWith('…')).toBe(true);
  });

  it('drops events whose category preference is disabled', () => {
    store.dispatch(setPreference({ category: 'agents', enabled: false }));
    __handleChatDoneForTests({ thread_id: 't', full_response: 'x' });
    expect(store.getState().notifications.items).toHaveLength(0);
    expect(showNativeNotification).not.toHaveBeenCalled();
  });

  it('skips the native banner when the window is focused', () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(true);
    __handleChatDoneForTests({ thread_id: 't', full_response: 'focused' });
    expect(showNativeNotification).not.toHaveBeenCalled();
  });

  it('fires the native banner when the window is unfocused', () => {
    vi.spyOn(document, 'hasFocus').mockReturnValue(false);
    __handleChatDoneForTests({ thread_id: 't', full_response: 'unfocused' });
    expect(showNativeNotification).toHaveBeenCalledTimes(1);
    expect(showNativeNotification).toHaveBeenCalledWith(
      expect.objectContaining({ title: 'Agent reply ready' })
    );
  });

  it('dispatches core_notification payloads with provided category and deep_link', () => {
    store.dispatch(setPreference({ category: 'system', enabled: true }));
    __handleCoreNotificationForTests({
      id: 'webhook:s:1',
      category: 'system',
      title: 'Webhook error',
      body: 'skill-x webhook returned HTTP 500',
      deep_link: '/webhooks',
      timestamp_ms: 1,
    });
    const items = store.getState().notifications.items;
    expect(items).toHaveLength(1);
    expect(items[0].id).toBe('webhook:s:1');
    expect(items[0].category).toBe('system');
    expect(items[0].deepLink).toBe('/webhooks');
  });

  it('ignores core_notification payloads missing id/title', () => {
    __handleCoreNotificationForTests({
      id: '',
      category: 'system',
      title: '',
      body: 'x',
      timestamp_ms: 1,
    });
    expect(store.getState().notifications.items).toHaveLength(0);
  });
});
