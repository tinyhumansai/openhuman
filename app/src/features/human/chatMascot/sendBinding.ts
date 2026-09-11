import debug from 'debug';

const bindingLog = debug('human:chat-mascot');

/**
 * How the mascot stage reaches the chat's send path.
 *
 * `Conversations` owns message sending (thread selection, attachments, queued
 * follow-ups, error surfacing) but the mascot stage lives in a *sibling* column
 * rendered by `pages/Accounts.tsx`, so the stage cannot receive these as props.
 */
export interface ChatMascotSendBinding {
  /** Route a transcript through the same path as a typed message. */
  submit: (text: string) => Promise<void> | void;
  /** Surface a mic/STT failure in the chat's own error banner. */
  onError: (message: string) => void;
  /** Whether sending is currently blocked (no thread yet, turn in flight, …). */
  disabled: boolean;
}

type Listener = () => void;

/**
 * A minimal external store for the send binding, read by the stage through
 * `useSyncExternalStore`.
 *
 * Deliberately NOT React context state: the binding's `disabled` flag flips
 * several times per turn, and putting it in the context value would re-render
 * every consumer — including the chat tree, whose per-frame reconciliation
 * during mascot lipsync is exactly the stall fixed in #5357. An external store
 * lets only the stage re-subscribe.
 */
export class ChatMascotSendStore {
  private binding: ChatMascotSendBinding | null = null;
  private listeners = new Set<Listener>();

  get = (): ChatMascotSendBinding | null => this.binding;

  subscribe = (listener: Listener): (() => void) => {
    this.listeners.add(listener);
    return () => {
      this.listeners.delete(listener);
    };
  };

  /**
   * Publish a new binding. No-ops when nothing observable changed so a
   * re-rendering publisher doesn't wake the stage for free.
   */
  set = (next: ChatMascotSendBinding | null): void => {
    const prev = this.binding;
    if (
      prev === next ||
      (prev != null &&
        next != null &&
        prev.submit === next.submit &&
        prev.onError === next.onError &&
        prev.disabled === next.disabled)
    ) {
      return;
    }
    bindingLog(
      '[chat-mascot][send-binding] update bound=%s disabled=%s',
      next != null,
      next?.disabled
    );
    this.binding = next;
    for (const listener of this.listeners) listener();
  };
}
