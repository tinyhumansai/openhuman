import { type AssistantState, useAui, useAuiState } from '@assistant-ui/react';
import debugFactory from 'debug';
import { useEffect } from 'react';

const debug = debugFactory('openhuman:chat-composer');

/**
 * Module-level so `useAuiState` can key its internal cache on selector
 * identity — an inline arrow would defeat that on every render. Mirrors
 * assistant-ui's own `useComposerInputValue`, which is what
 * `ComposerPrimitive.Input` renders from.
 */
const selectComposerText = (s: AssistantState) => (s.composer.isEditing ? s.composer.text : '');

/**
 * Keep assistant-ui's composer text and this app's `inputValue` prop in sync.
 *
 * `ComposerPrimitive.Input` is not a controlled React input: it renders
 * `aui.composer.text` and writes back through `aui.composer.setText`. This
 * app's composer text, by contrast, is owned by whichever surface renders the
 * composer — `Conversations` persists drafts from it, derives the inline
 * completion from it, and clears it after a send — and that ownership is not
 * something to hand to the primitive.
 *
 * So the two are bridged rather than one replacing the other, and the direction
 * of authority is deliberate: **the prop wins.** On every render where the two
 * disagree the prop is pushed into the composer store. A keystroke converges in
 * one pass — the primitive's own `onChange` sets the store synchronously
 * (`flushTapSync`) while our composed handler calls `setInputValue`, so by the
 * next render they already agree and this effect is a no-op. A surface that
 * *ignores* `setInputValue` (a read-only harness, a test with a stubbed setter)
 * gets the store reset back to the prop, which is exactly how the controlled
 * `<textarea>` this replaced behaved.
 *
 * The prop-wins direction is also what makes programmatic writes work: clearing
 * after send, seeding a slash command, restoring a draft. None of those go
 * through the textarea, so nothing would tell the composer store about them.
 */
export function useComposerTextBridge(value: string): void {
  const aui = useAui();
  const composerText = useAuiState(selectComposerText);

  useEffect(() => {
    if (composerText === value) return;
    debug(
      '[chat-composer] text bridge: prop -> composer store (propLen=%d storeLen=%d)',
      value.length,
      composerText.length
    );
    aui.composer.setText(value);
  }, [aui, composerText, value]);
}
