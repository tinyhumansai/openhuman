import { isTauri } from '@tauri-apps/api/core';
import { listen, type UnlistenFn } from '@tauri-apps/api/event';
import debug from 'debug';

import { store } from '../../store';
import {
  focusAccountFromNotification,
  noteWebviewNotificationFired,
} from '../../store/accountsSlice';
import { notificationReceived } from '../../store/notificationSlice';
import { WEBVIEW_NOTIFICATION_FIRED_EVENT, type WebviewNotificationFired } from './types';

const log = debug('webview-notifications');
const errLog = debug('webview-notifications:error');

let started = false;
let unlisten: UnlistenFn | null = null;

/**
 * Subscribe to `webview-notification:fired` events from the Tauri shell and
 * mirror each fire into Redux so the sidebar can bump an unread badge on
 * the originating account. Idempotent — subsequent calls are no-ops.
 */
export function startWebviewNotificationsService(): void {
  if (started) return;
  if (!isTauri()) {
    log('not running in tauri, skipping subscription');
    return;
  }
  started = true;

  listen<WebviewNotificationFired>(WEBVIEW_NOTIFICATION_FIRED_EVENT, event => {
    handleFired(event.payload);
  })
    .then(fn => {
      unlisten = fn;
      log('subscribed to %s', WEBVIEW_NOTIFICATION_FIRED_EVENT);
    })
    .catch(err => {
      errLog('failed to subscribe: %O', err);
      started = false;
    });
}

export function stopWebviewNotificationsService(): void {
  if (unlisten) {
    unlisten();
    unlisten = null;
  }
  started = false;
}

/**
 * Route a user-visible "click this notification" intent back to the
 * originating account — focuses it and clears the unread count. Safe to
 * call from in-app toast UIs or a future OS-notification click hook.
 */
export function handleNotificationClick(accountId: string): void {
  store.dispatch(focusAccountFromNotification({ accountId }));
}

function handleFired(payload: WebviewNotificationFired): void {
  const { account_id: accountId, provider, title, body, tag } = payload;
  log(
    'fired account=%s provider=%s title_chars=%d body_chars=%d',
    accountId,
    provider,
    title.length,
    body.length
  );
  store.dispatch(noteWebviewNotificationFired({ accountId }));
  const now = Date.now();
  store.dispatch(
    notificationReceived({
      id: `${accountId}:${tag ?? ''}:${now}`,
      category: 'messages',
      title,
      body,
      timestamp: now,
      read: false,
      accountId,
      provider,
      deepLink: `/accounts/${accountId}`,
    })
  );
}

/** Exposed for tests — resets module singletons between runs. */
export function __resetForTests(): void {
  started = false;
  unlisten = null;
}

/** Exposed for tests — dispatches as if a fired event arrived. */
export function __handleFiredForTests(payload: WebviewNotificationFired): void {
  handleFired(payload);
}
