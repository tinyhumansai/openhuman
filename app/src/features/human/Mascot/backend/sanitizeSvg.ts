// Minimal SVG sanitizer for backend-supplied mascot markup.
//
// `BackendMascot` injects per-state SVG via `dangerouslySetInnerHTML` and live
// `slot.innerHTML` swaps in the privileged Tauri renderer (which can reach the
// core RPC bearer). `innerHTML` never executes `<svg><script>`, but a
// `<foreignObject>` with an inline event handler — or an `href`/`xlink:href`
// pointing at `javascript:` — DOES execute under a CSP that permits inline
// handlers. We don't have DOMPurify in the dependency tree, so this strips the
// known SVG-in-HTML execution sinks before injection:
//   - `<script>` and `<foreignObject>` elements (and their content)
//   - inline event-handler attributes (`on*`)
//   - `javascript:`/`data:text/html` URLs in `href` / `xlink:href` / `src`
//
// This is intentionally conservative and string-based to match the existing
// pure-string render pipeline (`renderCore.ts`). It is defense-in-depth: the
// markup is still expected to come from the trusted backend manifest.

/** Elements whose mere presence in an HTML-parsed SVG can execute script. */
const FORBIDDEN_ELEMENT_RE = /<\s*(script|foreignObject)\b[\s\S]*?<\s*\/\s*\1\s*>/gi;
/** Self-closing or unterminated forbidden elements (`<script ... />`, `<foreignObject>`). */
const FORBIDDEN_ELEMENT_OPEN_RE = /<\s*(script|foreignObject)\b[^>]*>/gi;
/** Inline event handlers: `onload="…"`, `onerror='…'`, `onclick=…`. */
const EVENT_HANDLER_ATTR_RE = /\s+on[a-z]+\s*=\s*("[^"]*"|'[^']*'|[^\s>]+)/gi;
/** `href` / `xlink:href` / `src` carrying an executable scheme. */
const DANGEROUS_URL_ATTR_RE =
  /\s+(?:xlink:)?(?:href|src)\s*=\s*("\s*(?:javascript|data\s*:\s*text\/html)[^"]*"|'\s*(?:javascript|data\s*:\s*text\/html)[^']*'|\s*(?:javascript|data\s*:\s*text\/html)[^\s>]*)/gi;

/**
 * Strip script-execution sinks from an SVG fragment before it is injected via
 * `innerHTML` / `dangerouslySetInnerHTML`. Returns an empty string for empty
 * input. Pure — no DOM access — so it runs identically in tests and renderer.
 */
export function sanitizeSvg(svg: string): string {
  if (!svg) return '';
  return svg
    .replace(FORBIDDEN_ELEMENT_RE, '')
    .replace(FORBIDDEN_ELEMENT_OPEN_RE, '')
    .replace(EVENT_HANDLER_ATTR_RE, '')
    .replace(DANGEROUS_URL_ATTR_RE, '');
}
