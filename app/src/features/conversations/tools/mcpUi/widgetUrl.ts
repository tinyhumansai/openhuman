import { FILL_WIDTH } from './frameFit';
import type { McpUiResource } from './types';

/**
 * Sandbox for the proxy frame. The proxy keeps its own `ohwidget:` origin so a
 * widget can address it with `postMessage`; the widget itself runs in the
 * proxy's inner frame without `allow-same-origin`.
 */
export const PROXY_FRAME_SANDBOX = 'allow-scripts allow-same-origin allow-forms';

export const MIN_FRAME_WIDTH = 320;
export const FRAME_WIDTH_MARGIN = 8;

/** The sandbox proxy URL on the widget origin, carrying the declared CSP and theme. */
export function widgetProxyUrl(
  csp: McpUiResource['csp'] | undefined,
  windows: boolean,
  theme?: 'light' | 'dark'
): string {
  const base = windows ? 'http://ohwidget.localhost/proxy' : 'ohwidget://localhost/proxy';
  const query = new URLSearchParams();
  if (theme) query.set('theme', theme);
  for (const origin of csp?.connect_domains ?? []) query.append('connect', origin);
  for (const origin of csp?.resource_domains ?? []) query.append('resource', origin);
  const encoded = query.toString();
  return encoded ? `${base}?${encoded}` : base;
}

export function isWindowsHost(): boolean {
  return typeof navigator !== 'undefined' && /Windows/i.test(navigator.userAgent);
}

/** The frame's CSS width for a reported content width; the frame's max-width keeps it in the column. */
export function frameWidth(reported: number | null): string {
  if (reported === null || !Number.isFinite(reported) || reported >= FILL_WIDTH) return '100%';
  const width = Math.max(MIN_FRAME_WIDTH, Math.ceil(reported) + FRAME_WIDTH_MARGIN);
  return `${width}px`;
}
