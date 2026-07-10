/**
 * Theme codemod mapping table.
 *
 * Each PAIR entry is `[lightClass, darkClass, semanticClass]`. The codemod
 * collapses an adjacent `lightClass darkClass` (or the reversed order) into the
 * single semantic class — the CSS variable behind the semantic class handles
 * dark mode, so the `dark:` half is redundant after migration.
 *
 * Only high-confidence, audited pairings live here (see the className-pairing
 * audit). Opacity-suffixed variants (`bg-neutral-900/50`) are intentionally NOT
 * matched — the codemod's class boundaries exclude a trailing `/`, leaving them
 * untouched for manual handling.
 *
 * SINGLES are 1:1 class renames (no pairing).
 */

export const PAIRS = [
  // ── Backgrounds ─────────────────────────────────────────────────────────
  ['bg-white', 'dark:bg-neutral-900', 'bg-surface'],
  ['bg-white', 'dark:bg-neutral-800', 'bg-surface'],
  ['bg-white', 'dark:bg-stone-800', 'bg-surface'],
  ['bg-stone-50', 'dark:bg-neutral-800', 'bg-surface-muted'],
  ['bg-neutral-50', 'dark:bg-neutral-800', 'bg-surface-muted'],
  ['bg-stone-50', 'dark:bg-neutral-900', 'bg-surface-muted'],
  ['bg-stone-50', 'dark:bg-neutral-950', 'bg-surface-muted'],
  ['bg-stone-100', 'dark:bg-neutral-800', 'bg-surface-subtle'],
  ['bg-neutral-100', 'dark:bg-neutral-800', 'bg-surface-subtle'],
  ['bg-stone-200', 'dark:bg-neutral-800', 'bg-surface-strong'],
  ['bg-stone-200', 'dark:bg-neutral-700', 'bg-surface-strong'],
  ['bg-neutral-200', 'dark:bg-neutral-800', 'bg-surface-strong'],

  // ── Text ────────────────────────────────────────────────────────────────
  ['text-stone-900', 'dark:text-neutral-100', 'text-content'],
  ['text-stone-900', 'dark:text-neutral-50', 'text-content'],
  ['text-neutral-900', 'dark:text-neutral-100', 'text-content'],
  ['text-neutral-900', 'dark:text-neutral-50', 'text-content'],
  ['text-neutral-800', 'dark:text-neutral-100', 'text-content'],
  ['text-stone-800', 'dark:text-neutral-100', 'text-content'],
  ['text-stone-800', 'dark:text-neutral-200', 'text-content'],
  ['text-neutral-800', 'dark:text-neutral-200', 'text-content'],
  ['text-stone-700', 'dark:text-neutral-200', 'text-content-secondary'],
  ['text-stone-700', 'dark:text-neutral-300', 'text-content-secondary'],
  ['text-neutral-700', 'dark:text-neutral-200', 'text-content-secondary'],
  ['text-neutral-700', 'dark:text-neutral-300', 'text-content-secondary'],
  ['text-stone-600', 'dark:text-neutral-300', 'text-content-secondary'],
  ['text-neutral-600', 'dark:text-neutral-300', 'text-content-secondary'],
  ['text-stone-600', 'dark:text-neutral-400', 'text-content-secondary'],
  ['text-stone-500', 'dark:text-neutral-400', 'text-content-muted'],
  ['text-neutral-500', 'dark:text-neutral-400', 'text-content-muted'],
  ['text-stone-400', 'dark:text-neutral-500', 'text-content-faint'],
  ['text-neutral-400', 'dark:text-neutral-500', 'text-content-faint'],

  // ── Borders ─────────────────────────────────────────────────────────────
  ['border-stone-200', 'dark:border-neutral-800', 'border-line'],
  ['border-neutral-200', 'dark:border-neutral-800', 'border-line'],
  ['border-stone-200', 'dark:border-neutral-700', 'border-line'],
  ['border-stone-100', 'dark:border-neutral-800', 'border-line-subtle'],
  ['border-neutral-100', 'dark:border-neutral-800', 'border-line-subtle'],
  ['border-stone-300', 'dark:border-neutral-700', 'border-line-strong'],
  ['border-neutral-300', 'dark:border-neutral-700', 'border-line-strong'],

  // ── Opacity-suffixed surface pairs (pass 2) ─────────────────────────────
  // The dark half carries an opacity suffix, so the boundary-based matcher skips
  // them unless the full literal (incl. /NN) is listed. Light stays opaque; dark
  // collapses to the opaque themed surface (the translucency was a dark-mode
  // aesthetic that read as grey under custom dark themes like Midnight).
  ['bg-stone-50', 'dark:bg-neutral-800/60', 'bg-surface-muted'],
  ['bg-neutral-50', 'dark:bg-neutral-800/60', 'bg-surface-muted'],
  ['bg-stone-50', 'dark:bg-neutral-800/40', 'bg-surface-muted'],
  ['bg-stone-50', 'dark:bg-neutral-900/50', 'bg-surface-muted'],
  ['bg-stone-50', 'dark:bg-neutral-900/40', 'bg-surface-muted'],
  ['bg-stone-100', 'dark:bg-neutral-800/60', 'bg-surface-subtle'],
  ['bg-neutral-100', 'dark:bg-neutral-800/60', 'bg-surface-subtle'],
  ['bg-stone-200', 'dark:bg-neutral-700', 'bg-surface-strong'],
  ['bg-stone-300', 'dark:bg-neutral-600', 'bg-surface-strong'],
  ['bg-stone-300', 'dark:bg-neutral-700', 'bg-surface-strong'],
  ['bg-neutral-300', 'dark:bg-neutral-600', 'bg-surface-strong'],
  ['bg-white', 'dark:bg-neutral-900/40', 'bg-surface'],
  ['bg-white', 'dark:bg-neutral-900/30', 'bg-surface'],
  ['bg-white', 'dark:bg-neutral-600', 'bg-surface'],
  ['bg-white', 'dark:bg-neutral-700', 'bg-surface'],

  // ── Placeholder text ────────────────────────────────────────────────────
  ['placeholder-stone-400', 'dark:placeholder-neutral-500', 'placeholder-content-faint'],
  ['placeholder-neutral-400', 'dark:placeholder-neutral-500', 'placeholder-content-faint'],
  ['placeholder-stone-500', 'dark:placeholder-neutral-500', 'placeholder-content-faint'],

  // Translucent panels — opacity intended in BOTH modes (sticky headers, glass).
  // Preserve the opacity on the themed surface so they stay translucent.
  ['bg-white/95', 'dark:bg-neutral-900/95', 'bg-surface/95'],
  ['bg-white/90', 'dark:bg-neutral-900/90', 'bg-surface/90'],
  ['bg-white/80', 'dark:bg-neutral-900/80', 'bg-surface/80'],
  ['bg-white/70', 'dark:bg-neutral-950/40', 'bg-surface/70'],
  ['bg-white/40', 'dark:bg-neutral-900/40', 'bg-surface/40'],

  // ── Hover / interaction states ──────────────────────────────────────────
  ['hover:bg-stone-50', 'dark:hover:bg-neutral-800', 'hover:bg-surface-hover'],
  ['hover:bg-stone-50', 'dark:hover:bg-neutral-800/60', 'hover:bg-surface-hover'],
  ['hover:bg-stone-100', 'dark:hover:bg-neutral-800/60', 'hover:bg-surface-hover'],
  ['hover:bg-neutral-50', 'dark:hover:bg-neutral-800/60', 'hover:bg-surface-hover'],
  ['hover:bg-stone-100', 'dark:hover:bg-neutral-800', 'hover:bg-surface-hover'],
  ['hover:bg-neutral-50', 'dark:hover:bg-neutral-800', 'hover:bg-surface-hover'],
  ['hover:bg-neutral-100', 'dark:hover:bg-neutral-800', 'hover:bg-surface-hover'],
  ['hover:text-stone-700', 'dark:hover:text-neutral-200', 'hover:text-content-secondary'],
  ['hover:text-stone-600', 'dark:hover:text-neutral-300', 'hover:text-content-secondary'],
  ['hover:text-stone-900', 'dark:hover:text-neutral-100', 'hover:text-content'],
  ['hover:text-stone-800', 'dark:hover:text-neutral-100', 'hover:text-content'],
];

export const SINGLES = [
  // Font role clarity: font-display is an alias of font-title (both → --font-title).
  ['font-display', 'font-title'],
];
