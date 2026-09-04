import { openUrl } from './openUrl';

/**
 * True when `href` would take the main webview away from the app itself.
 *
 * The desktop shell is a single webview with no browser chrome — no back
 * button, no address bar — so any top-level navigation to a remote page is
 * one-way: the chat is gone until the app is restarted. Only the app's own
 * origin (its hash routes, `#/chat`, `#/settings/...`) is safe to follow.
 *
 * `about:`/`blob:`/`data:` and in-page anchors are left alone: they are not
 * remote pages, and a `data:` preview is a deliberate in-app render.
 */
export function isExternalNavigation(href: string, appOrigin: string): boolean {
  const trimmed = href.trim();
  if (!trimmed || trimmed.startsWith('#')) return false;
  let url: URL;
  try {
    url = new URL(trimmed, appOrigin);
  } catch {
    return false;
  }
  if (url.protocol !== 'http:' && url.protocol !== 'https:') return false;
  return url.origin !== appOrigin;
}

/**
 * Install a document-level, capture-phase guard that keeps a link click from
 * navigating the main webview away from the app.
 *
 * Chat bubbles already route their links through `openUrl` (see
 * `AgentMessageBubble`'s `MarkdownAnchor`), but that is one component's
 * discipline, and every other rendered anchor — tool output, a panel, raw
 * HTML inside a message — inherits the webview's default behaviour instead.
 * When one of those is clicked the shell navigates and the user is stranded
 * on the page with no way back, which is what this guard exists to prevent.
 *
 * It listens in the BUBBLE phase, deliberately. In the capture phase this
 * document-level listener would run *before* the component's own handler, so
 * a chat link would be opened once here and again by `MarkdownAnchor` — two
 * browser tabs for one click. Bubbling lets the owning component go first;
 * anything it already called `preventDefault` on is skipped, and the default
 * navigation has still not happened by the time this runs, so preventing it
 * here is not too late.
 *
 * Returns the teardown function.
 */
export function installExternalLinkGuard(doc: Document = document): () => void {
  const onClick = (event: MouseEvent) => {
    if (event.defaultPrevented || event.button !== 0) return;
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return;

    const target = event.target as Element | null;
    const anchor = target?.closest?.('a[href]') as HTMLAnchorElement | null;
    if (!anchor) return;

    const href = anchor.getAttribute('href') ?? '';
    if (!isExternalNavigation(href, doc.location.origin)) return;

    event.preventDefault();
    void openUrl(anchor.href).catch(() => {
      // The OS handler refused; staying in the app beats a one-way navigation.
    });
  };

  doc.addEventListener('click', onClick);
  return () => doc.removeEventListener('click', onClick);
}
