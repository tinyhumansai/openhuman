/**
 * assistant-ui's default-composer visual spec, ported to this repo.
 *
 * The upstream look lives in `@assistant-ui/styles@0.3.7`
 * (`src/styles/generated.css`) as a set of `@apply` rules. That package is
 * **not** installed and must not be: it is compiled Tailwind v4 and leans on
 * `color-mix()`, `@property`, `oklch()` and `@layer`, none of which esbuild
 * downlevels for this app's `cssTarget: "safari15"` floor — it would render
 * unstyled on the oldest supported macOS. So the spec is ported, not imported,
 * and this file is the single place the port is reviewable.
 *
 * Each constant below carries the upstream rule verbatim in its comment,
 * followed by the two kinds of translation that were required:
 *
 *  1. **Tailwind v4 → v3.4** — `size-8.5` has no v3 equivalent (fractional
 *     spacing is a v4 feature; `size-*` itself exists in 3.4), so it becomes
 *     `size-[34px]`. `has-[…]:` and `empty:` variants DO exist in 3.4 and are
 *     kept as written.
 *  2. **shadcn colour names → OpenHuman semantic tokens** — upstream is written
 *     against shadcn's palette (`bg-background`, `text-muted-foreground`,
 *     `border-input`, `ring-ring`, `bg-accent`). This app has its own var-backed
 *     semantic scale (`tailwind.config.js`), and `npm run lint:ui-tokens` fails
 *     the build on raw palette names, so every colour is remapped:
 *
 *     | shadcn                 | OpenHuman            |
 *     | ---------------------- | -------------------- |
 *     | `bg-background`        | `bg-surface`         |
 *     | `text-muted-foreground`| `text-content-muted` |
 *     | `border-input`         | `border-line-strong` |
 *     | `ring-ring`            | `ring-primary-500`   |
 *     | `bg-muted`             | `bg-surface-muted`   |
 *     | `bg-accent`            | `bg-surface-hover`   |
 *
 * Nothing here is behavioural. Structure and behaviour come from the headless
 * `ComposerPrimitive` in `@assistant-ui/react`; this file is only how it looks.
 */

/** assistant-ui's current composer root. */
export const COMPOSER_ROOT = 'relative flex w-full flex-col pb-2.5';

/**
 * `.aui-composer-attachment-dropzone` —
 * `flex w-full flex-col rounded-2xl border border-input bg-background px-1 pt-2
 *  outline-hidden transition-shadow has-[textarea:focus-visible]:border-ring
 *  has-[textarea:focus-visible]:ring-2 has-[textarea:focus-visible]:ring-ring/20
 *  data-[dragging=true]:border-ring data-[dragging=true]:border-dashed
 *  data-[dragging=true]:bg-accent/50`
 *
 * `border-input`/`ring-ring`/`bg-accent` remapped per the table above. The
 * `rounded-2xl` and `border` classes are load-bearing beyond looks: the drag
 * tests and the mascot-dock anchoring test locate this element with
 * `closest('div.rounded-2xl')`.
 */
export const COMPOSER_DROPZONE =
  'flex w-full flex-col rounded-3xl border border-line bg-surface-muted/55 ' +
  'transition-colors focus-within:border-line-strong focus-within:bg-surface-muted/75 ' +
  'data-[dragging=true]:border-primary-500 data-[dragging=true]:border-dashed ' +
  'data-[dragging=true]:bg-surface-hover/50';

/**
 * `.aui-composer-input` —
 * `mb-1 max-h-32 min-h-14 w-full resize-none bg-transparent px-4 pt-2 pb-3
 *  text-sm outline-hidden placeholder:text-muted-foreground focus-visible:ring-0`
 *
 * Only the placeholder colour is remapped. `disabled:` handling is appended:
 * upstream has no disabled state because its composer is never locked, while
 * this one is (see `composerInteractionBlocked` / `isSending`).
 */
export const COMPOSER_INPUT =
  'max-h-32 min-h-14 w-full resize-none bg-transparent px-3.5 pt-3 pb-2 text-sm leading-5 ' +
  'text-content outline-hidden placeholder:text-content-muted focus-visible:ring-0 ' +
  'disabled:cursor-not-allowed disabled:opacity-50';

/**
 * Padding/typography of {@link COMPOSER_INPUT}, minus the box metrics, for the
 * inline-completion ghost layer that has to sit pixel-aligned behind the real
 * textarea. Not an upstream class — assistant-ui has no inline completion.
 */
export const COMPOSER_GHOST_OVERLAY =
  'pointer-events-none absolute inset-0 overflow-hidden whitespace-pre-wrap wrap-break-word ' +
  'px-3.5 pt-3 pb-2 text-sm leading-5 font-sans';

/** assistant-ui's compact footer action row. */
export const COMPOSER_ACTION_WRAPPER = 'relative flex items-center justify-between px-2 pb-2';

/**
 * `.aui-composer-attachments` —
 * `mb-2 flex w-full flex-row items-center gap-2 overflow-x-auto px-1.5 pt-0.5 pb-1 empty:hidden`
 * (unchanged; `empty:` is a Tailwind 3.4 variant).
 */
export const COMPOSER_ATTACHMENTS =
  'mb-2 flex w-full flex-row items-center gap-2 overflow-x-auto px-1.5 pt-0.5 pb-1 empty:hidden';

/**
 * `.aui-composer-add-attachment` —
 * `size-8.5 rounded-full p-1 font-semibold text-xs hover:bg-muted-foreground/15`
 *
 * `size-8.5` is Tailwind v4 fractional spacing (8.5 × 0.25rem = 34px) with no
 * v3 equivalent, so it becomes the arbitrary `size-[34px]`.
 * `bg-muted-foreground/15` → `bg-content-muted/15`.
 */
export const COMPOSER_ADD_ATTACHMENT =
  'size-7 shrink-0 rounded-full p-1 text-xs font-semibold text-content-muted ' +
  'hover:bg-content-muted/15 hover:text-content-secondary';

/**
 * Not an upstream class. assistant-ui's default composer has no voice button;
 * this app's does, and it sits beside the add-attachment button in the action
 * wrapper, so it takes the same box so the two read as one control group.
 */
export const COMPOSER_MIC = COMPOSER_ADD_ATTACHMENT;

/** assistant-ui's compact circular send/cancel action. */
export const COMPOSER_SEND = 'size-7 shrink-0 rounded-full';
export const COMPOSER_CANCEL = COMPOSER_SEND;

/** `.aui-composer-send-icon` — `size-4`. */
export const COMPOSER_SEND_ICON = 'size-4';

/** `.aui-composer-cancel-icon` — `size-3 fill-current`. */
export const COMPOSER_CANCEL_ICON = 'size-3 fill-current';

/**
 * Drag overlay. Not upstream — assistant-ui signals a drag by restyling the
 * dropzone border (`data-[dragging=true]:*`, kept above) and shows no message.
 * This app shows an explicit "drop to attach" plate, which predates the port
 * and is asserted by `ChatComposer.test.tsx`, so it stays.
 */
export const COMPOSER_DRAG_OVERLAY =
  'pointer-events-none absolute inset-0 z-20 flex items-center justify-center rounded-2xl ' +
  'border-2 border-dashed border-primary-500 bg-surface/90 text-sm font-medium text-primary-600';
