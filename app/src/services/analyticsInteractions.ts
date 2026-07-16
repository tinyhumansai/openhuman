import type { AnalyticsParams } from './analytics';
import { currentAppPath, currentPageHash } from './analyticsRoutes';

const INTERACTIVE_CLICK_SELECTOR =
  'button,a,[role="button"],summary,[data-track],[data-analytics-id],[data-testid],[data-walkthrough]';
const CONTROL_CHANGE_SELECTOR =
  'select,input[type="checkbox"],input[type="radio"],input[type="range"],[role="switch"],[role="checkbox"],[role="radio"]';

type InteractionEventName = 'ui_click' | 'ui_control_change' | 'ui_form_submit';
type InteractionTracker = (eventName: InteractionEventName, params: AnalyticsParams) => void;

let trackingStarted = false;

/** Install app-wide delegated listeners for privacy-safe UI interaction events. */
export function startInteractionTracking(track: InteractionTracker): () => void {
  if (trackingStarted || typeof document === 'undefined') return () => undefined;
  trackingStarted = true;

  const handleClick = (event: MouseEvent) => {
    const target = event.target instanceof Element ? event.target : null;
    const element = target?.closest(INTERACTIVE_CLICK_SELECTOR);
    if (!(element instanceof HTMLElement) || shouldSkipInteractionElement(element)) return;

    track('ui_click', {
      ...interactionBaseProperties(element),
      interaction_kind: 'click',
      control_id: controlIdentifier(element),
      destination: destinationForElement(element),
    });
  };

  const handleChange = (event: Event) => {
    const target = event.target instanceof Element ? event.target : null;
    const element = target?.closest(CONTROL_CHANGE_SELECTOR);
    if (!(element instanceof HTMLElement) || shouldSkipInteractionElement(element)) return;

    track('ui_control_change', {
      ...interactionBaseProperties(element),
      interaction_kind: 'change',
      control_id: controlIdentifier(element),
      control_state: controlState(element),
    });
  };

  const handleSubmit = (event: SubmitEvent) => {
    const form = event.target instanceof HTMLFormElement ? event.target : null;
    if (!form || shouldSkipInteractionElement(form)) return;

    track('ui_form_submit', {
      ...interactionBaseProperties(form),
      interaction_kind: 'submit',
      control_id: controlIdentifier(form),
    });
  };

  document.addEventListener('click', handleClick, true);
  document.addEventListener('change', handleChange, true);
  document.addEventListener('submit', handleSubmit, true);

  return () => {
    document.removeEventListener('click', handleClick, true);
    document.removeEventListener('change', handleChange, true);
    document.removeEventListener('submit', handleSubmit, true);
    trackingStarted = false;
  };
}

function interactionBaseProperties(element: HTMLElement): AnalyticsParams {
  return {
    page: currentAppPath(),
    page_hash: currentPageHash(),
    element_tag: element.tagName.toLowerCase(),
    element_role: scrubIdentifier(element.getAttribute('role')) ?? '',
    element_type: scrubIdentifier(element.getAttribute('type')) ?? '',
  };
}

function controlIdentifier(element: HTMLElement): string {
  const explicit =
    element.getAttribute('data-analytics-id') ??
    element.getAttribute('data-track') ??
    element.getAttribute('data-testid') ??
    element.getAttribute('data-walkthrough') ??
    element.getAttribute('name') ??
    element.id;
  const scrubbed = scrubIdentifier(explicit);
  if (scrubbed) return scrubbed;

  const hrefDestination = destinationForElement(element);
  if (hrefDestination) return `link_${scrubIdentifier(hrefDestination) ?? 'internal'}`;

  const container = nearestStableContainer(element);
  const tag = element.tagName.toLowerCase();
  if (container) return `${tag}_in_${container}`;
  // Every native/shared button is still captured even when its call site has
  // not yet received a semantic data-analytics-id. Keep those controls
  // distinguishable without reading their text/aria-label (both can contain
  // user content) by assigning a page-local, DOM-order fallback.
  const peers = Array.from(document.querySelectorAll(INTERACTIVE_CLICK_SELECTOR)).filter(
    peer => peer instanceof HTMLElement && !shouldSkipInteractionElement(peer)
  );
  const position = peers.indexOf(element);
  console.debug('[analytics] controlIdentifier fallback', { tag, position });
  return position >= 0 ? `${tag}_${position + 1}` : tag;
}

function destinationForElement(element: HTMLElement): string {
  const href = element instanceof HTMLAnchorElement ? element.getAttribute('href') : null;
  if (!href) return '';
  if (href.startsWith('#/')) return href.slice(1);
  if (href.startsWith('/')) return href;
  return href.startsWith('http') ? 'external' : '';
}

function controlState(element: HTMLElement): string {
  if (element instanceof HTMLInputElement) {
    if (element.type === 'checkbox' || element.type === 'radio') {
      return element.checked ? 'checked' : 'unchecked';
    }
    if (element.type === 'range') return 'changed';
  }
  if (element instanceof HTMLSelectElement) return 'selected';

  const ariaChecked = element.getAttribute('aria-checked');
  if (ariaChecked === 'true' || ariaChecked === 'false' || ariaChecked === 'mixed') {
    return ariaChecked;
  }
  return 'changed';
}

function nearestStableContainer(element: HTMLElement): string | undefined {
  const container = element.closest('[data-testid],[data-walkthrough],[data-analytics-id]');
  if (!(container instanceof HTMLElement) || container === element) return undefined;
  return scrubIdentifier(
    container.getAttribute('data-analytics-id') ??
      container.getAttribute('data-testid') ??
      container.getAttribute('data-walkthrough')
  );
}

function shouldSkipInteractionElement(element: HTMLElement): boolean {
  if (element.closest('[data-analytics-skip="true"],[data-no-analytics="true"]')) return true;
  if (element.closest('[contenteditable="true"]')) return true;
  if (element instanceof HTMLInputElement) {
    return ['text', 'search', 'email', 'password', 'tel', 'url', 'number', 'file'].includes(
      element.type
    );
  }
  if (element instanceof HTMLTextAreaElement) return true;
  return false;
}

function scrubIdentifier(value: string | null | undefined): string | undefined {
  const trimmed = value?.trim();
  if (!trimmed) return undefined;
  const withoutQuery = trimmed.split(/[?#]/)[0] ?? trimmed;
  const scrubbed = withoutQuery
    .replace(/[A-Z0-9._%+-]+@[A-Z0-9.-]+\.[A-Z]{2,}/gi, ':email')
    .replace(/[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}/gi, ':id')
    .replace(/\b[0-9a-f]{16,}\b/gi, ':id')
    .replace(/\b\d{3,}\b/g, ':num')
    .replace(/[^a-zA-Z0-9:_/-]+/g, '_')
    .replace(/_+/g, '_')
    .replace(/^_+|_+$/g, '')
    .toLowerCase()
    .slice(0, 80);
  return scrubbed || undefined;
}
