/**
 * Puts text in the chat composer without sending it. A widget's
 * `ui/message` lands here so the user reads and sends it themselves.
 */
export const COMPOSER_PREFILL_EVENT = 'openhuman:composer-prefill';

const MAX_PREFILL_CHARS = 4000;

export function requestComposerPrefill(text: string): void {
  const value = text.slice(0, MAX_PREFILL_CHARS);
  window.dispatchEvent(new CustomEvent(COMPOSER_PREFILL_EVENT, { detail: { text: value } }));
}

/** Subscribes `apply` to prefill requests; returns the unsubscribe. */
export function onComposerPrefill(apply: (text: string) => void): () => void {
  const listener = (event: Event) => {
    const text = (event as CustomEvent<{ text?: unknown }>).detail?.text;
    if (typeof text === 'string' && text.trim()) apply(text);
  };
  window.addEventListener(COMPOSER_PREFILL_EVENT, listener);
  return () => window.removeEventListener(COMPOSER_PREFILL_EVENT, listener);
}
