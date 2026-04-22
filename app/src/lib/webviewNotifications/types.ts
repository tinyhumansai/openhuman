/**
 * Shape of the `webview-notification:fired` Tauri event payload emitted by
 * the Rust shell whenever an embedded webview renderer creates a native
 * notification. Mirror of `WebviewNotificationFired` in
 * `app/src-tauri/src/webview_accounts/mod.rs` — keep the two in sync.
 */
export interface WebviewNotificationFired {
  account_id: string;
  provider: string;
  title: string;
  body: string;
  tag?: string | null;
}

export const WEBVIEW_NOTIFICATION_FIRED_EVENT = 'webview-notification:fired';
