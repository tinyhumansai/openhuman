import { useAui } from '@assistant-ui/react';
import debugFactory from 'debug';
import { type RefObject, useEffect } from 'react';

const debug = debugFactory('openhuman:assistant-composer');

const MODAL_SELECTOR = '[role="dialog"], [role="alertdialog"], dialog, [aria-modal="true"]';

function isEditable(el: Element): boolean {
  if (el instanceof HTMLInputElement || el instanceof HTMLTextAreaElement) return true;
  if (el instanceof HTMLSelectElement) return true;
  // `isContentEditable` is unimplemented in jsdom; the attribute is what Lexical sets.
  return el.closest('[contenteditable="true"]') !== null;
}

/**
 * Puts the caret at the end of the composer when a conversation opens: once when
 * the composer mounts (a thread was opened) and once per `threads.selectionChanged`
 * (a switch, including New chat landing on another thread). Every open path —
 * thread list, route, New chat — converges on one of those two, and nothing else
 * (messages, re-renders) triggers it.
 *
 * The plain `ComposerPrimitive.Input` does this natively; `LexicalComposerInput`
 * only focuses on `thread.runStart`, hence this hook.
 *
 * It never steals focus: it stands down while a modal is open, while focus is
 * already in another input, and while the composer is disabled.
 */
export function useComposerFocusOnThreadOpen(inputWrapperRef: RefObject<HTMLElement | null>) {
  const aui = useAui();

  useEffect(() => {
    const focus = (reason: 'open' | 'switch') => {
      // The same condition `LexicalComposerInput` makes the editor read-only on.
      const disabled =
        aui.thread.getState().isDisabled || aui.composer.getState().dictation?.inputDisabled;
      const textbox = inputWrapperRef.current?.querySelector<HTMLElement>(
        '[contenteditable="true"]'
      );
      if (disabled || !textbox) {
        debug('focus on %s: skipped, composer disabled or not mounted', reason);
        return;
      }
      const active = document.activeElement;
      const elsewhere = active && active !== textbox && active !== document.body;
      if (
        (elsewhere && (isEditable(active) || active.closest(MODAL_SELECTOR))) ||
        document.querySelector('[aria-modal="true"]')
      ) {
        debug('focus on %s: skipped, a modal or another input holds focus', reason);
        return;
      }

      textbox.focus({ preventScroll: true });
      // Caret after a restored draft, not before it. An empty composer needs no
      // caret placement, and an explicit DOM selection there is not inert:
      // Lexical resolves its selection from the DOM on `compositionstart`, and
      // a range on the empty paragraph makes it insert its composition-start
      // character, which `SyncPlugin` writes to the store past the bridge's
      // IME gate. Where `beforeinput` is missing (jsdom), that store write
      // surfaced as a phantom draft mid-composition.
      const selection = textbox.textContent ? window.getSelection() : null;
      if (selection) {
        selection.selectAllChildren(textbox);
        selection.collapseToEnd();
      }
      debug('focus on %s: composer focused', reason);
    };

    focus('open');
    return aui.on('threads.selectionChanged', () => focus('switch'));
  }, [aui, inputWrapperRef]);
}
