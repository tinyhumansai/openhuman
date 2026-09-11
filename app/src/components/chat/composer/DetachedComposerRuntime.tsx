import {
  AssistantRuntimeProvider,
  type ThreadMessageLike,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import type { ReactNode } from 'react';

const NO_MESSAGES: ThreadMessageLike[] = [];

/**
 * An assistant-ui runtime for a composer surface that owns **no chat thread**.
 *
 * `ChatComposer` is built on `ComposerPrimitive`, which resolves its runtime
 * from React context. A surface that renders inside the app shell and mounts no
 * runtime of its own therefore inherits the app-wide one from
 * `ChatRuntimeProvider` — which is bound to `state.thread.selectedThreadId`,
 * the HOME chat's thread. For a surface whose conversation is not a chat thread
 * at all, that inheritance points its composer at an unrelated conversation.
 * That is a data-loss-shaped mistake, not a cosmetic one.
 *
 * The obvious fix — `AssistantUiRuntimeProvider threadId={null}` — is the right
 * shape but the wrong dependency: that provider reads Redux unconditionally
 * (`useAppSelector` for the selected thread, and again inside
 * `useOpenHumanExternalStore`), so it requires a store even when the thread is
 * explicitly `null` and there is nothing to select. A surface with no thread has
 * nothing to project from the store either, so this mounts a genuinely inert
 * runtime instead: no messages, never running, and a `onNew` that fails loudly
 * because nothing should reach it.
 *
 * Nothing routes a send through this runtime. `ChatComposer` intercepts
 * `ComposerPrimitive.Root`'s submit and calls its own `onSend` prop (see that
 * component's doc comment for why the primitive's send path cannot be used).
 * What the runtime actually supplies here is the composer's text store, which
 * `useComposerTextBridge` mirrors the host's `inputValue` into.
 */
export function DetachedComposerRuntime({ children }: { children: ReactNode }) {
  const runtime = useExternalStoreRuntime<ThreadMessageLike>({
    messages: NO_MESSAGES,
    isRunning: false,
    convertMessage: m => m,
    onNew: async () => {
      // Unreachable by construction. Loud rather than silent: a silent no-op
      // here would look exactly like a dropped message.
      throw new Error(
        'DetachedComposerRuntime: this surface owns no chat thread, so sends must go through ' +
          "the composer's own onSend prop, not the assistant-ui runtime."
      );
    },
  });
  return <AssistantRuntimeProvider runtime={runtime}>{children}</AssistantRuntimeProvider>;
}

export default DetachedComposerRuntime;
