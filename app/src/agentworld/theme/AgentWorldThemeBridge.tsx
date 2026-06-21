/**
 * AgentWorldThemeBridge — maps OpenHuman design tokens to tiny.place CSS variables.
 *
 * Injects a `<style>` block that bridges OpenHuman's Tailwind tokens (ocean
 * primary `#4A83DD`, sage/amber/coral semantics) to the CSS variables the
 * vendored tiny.place components expect.  This avoids modifying the vendored
 * source: we theme-bridge at the boundary.
 *
 * For Wave 0 this is a minimal stub.  Full token mapping will be expanded when
 * the vendored Explore UI is ported in Wave 1.
 */

const THEME_BRIDGE_CSS = `
  :root {
    /* tiny.place primary → OpenHuman ocean */
    --tp-color-primary: #4A83DD;
    --tp-color-primary-hover: #3a73cd;
    /* tiny.place surface → OpenHuman dark background */
    --tp-color-surface: #111827;
    --tp-color-surface-secondary: #1f2937;
    /* tiny.place text */
    --tp-color-text-primary: #f9fafb;
    --tp-color-text-secondary: #9ca3af;
    /* tiny.place semantic */
    --tp-color-success: #10b981;
    --tp-color-warning: #f59e0b;
    --tp-color-error: #ef4444;
  }
`;

export default function AgentWorldThemeBridge() {
  return <style>{THEME_BRIDGE_CSS}</style>;
}
