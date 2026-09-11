import { MessagePartPrimitive, TextMessagePartProvider } from '@assistant-ui/react';

export interface AuiPlainTextProps {
  /** The text to render. Plain text — never Markdown. */
  text: string;
  /** True while the text is still growing, which drives the caret below. */
  isRunning?: boolean;
  /** Token classes for the rendered `<span>`. */
  className?: string;
  /** Rendered only while `isRunning`, via `MessagePartPrimitive.InProgress`. */
  caret?: React.ReactNode;
}

/**
 * Plain text rendered through assistant-ui's HEADLESS message-part primitives.
 *
 * `TextMessagePartProvider` synthesises a `part` scope from a bare string, so
 * `MessagePartPrimitive.Text` and `MessagePartPrimitive.InProgress` work here
 * without a thread runtime, a message index, or a `MessagePrimitive.Root`
 * ancestor. That matters: the transcript is mounted both under a runtime (the
 * app) and without one (several unit tests), and this component must render
 * identically in both.
 *
 * Why the caret is a primitive rather than a `{isRunning && ...}`: the running
 * state now lives in the part scope, so a single source decides both the text
 * and whether it is still arriving. There is no way for the two to disagree.
 *
 * `smooth` is OFF deliberately. assistant-ui's smooth reveal animates text in
 * over `requestAnimationFrame` ticks; on the live streaming tail that would add
 * a render per frame on top of a render per token, in the exact hot path
 * `ChatThreadView.renderPerf.test.tsx` exists to protect.
 *
 * Styling is OpenHuman semantic tokens only — the primitives ship no CSS, which
 * is the whole reason the headless layer was chosen over `@assistant-ui/styles`
 * (compiled Tailwind v4: `color-mix()` / `oklch()` / `@property`, none of which
 * esbuild downlevels for this repo's `safari15` CSS target).
 */
export function AuiPlainText({ text, isRunning = false, className, caret }: AuiPlainTextProps) {
  return (
    <TextMessagePartProvider text={text} isRunning={isRunning}>
      <MessagePartPrimitive.Text smooth={false} className={className} />
      {caret ? <MessagePartPrimitive.InProgress>{caret}</MessagePartPrimitive.InProgress> : null}
    </TextMessagePartProvider>
  );
}
