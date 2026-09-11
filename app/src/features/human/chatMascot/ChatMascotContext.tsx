import debug from 'debug';
import {
  createContext,
  type ReactNode,
  type RefObject,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
} from 'react';

import { useAppDispatch } from '../../../store/hooks';
import { setChatMascotExpanded } from '../../../store/mascotSlice';
import { type ChatMascotSendBinding, ChatMascotSendStore } from './sendBinding';

const mascotLog = debug('human:chat-mascot');

/**
 * Shared handles for the merged chat surface's mascot.
 *
 * Every field is **stable for the provider's lifetime** — refs, a store
 * instance, and dispatch-bound callbacks. Nothing here changes identity, so
 * consuming this context never re-renders a component on its own. Reactive
 * state lives elsewhere on purpose:
 *
 *  - `expanded` / `speakReplies` / `listening` → Redux (`mascotSlice`), read by
 *    the few components that actually need them.
 *  - the chat send binding → `sendStore`, read via `useSyncExternalStore`.
 *
 * That split is load-bearing: `Conversations` consumes this context to render
 * the dock, and the mascot re-renders at ~60fps during TTS lipsync. A reactive
 * context value would reconcile the whole chat tree every frame, which is the
 * stall that #5357 had to fix on the Human page.
 */
export interface ChatMascotContextValue {
  /** The small mascot slot standing on the composer. Supplies the docked rect. */
  dockRef: RefObject<HTMLElement | null>;
  /**
   * Ref callback the dock passes to its element. Keeps `dockRef` in sync and
   * publishes the node on `ChatMascotDockNodeContext` — see `useDockNode`.
   */
  setDockNode: (node: HTMLElement | null) => void;
  /** The large square inside the voice stage. Supplies the expanded rect. */
  stageRef: RefObject<HTMLElement | null>;
  /** How the stage reaches the chat's send path. */
  sendStore: ChatMascotSendStore;
  expand: () => void;
  collapse: () => void;
}

const ChatMascotContext = createContext<ChatMascotContextValue | null>(null);

/**
 * The dock element, published separately from the main context.
 *
 * A ref alone cannot drive the overlay's `ResizeObserver`: refs are stable, so
 * an effect keyed on one runs once and reads whatever `dockRef.current` happened
 * to be at that moment. The dock can mount *after* the overlay (the case the
 * anchor poll exists for) or remount with the composer — in both cases the
 * observer would end up watching nothing, or watching detached nodes.
 *
 * It lives in its own context rather than on `ChatMascotContextValue` because
 * that value must stay referentially stable: `Conversations` consumes it, and
 * re-rendering the chat tree every time the dock mounts would chip away at the
 * guarantee documented above. Only the overlay subscribes here.
 */
const ChatMascotDockNodeContext = createContext<HTMLElement | null>(null);

/**
 * Mounted by the merged chat surface (`pages/Accounts.tsx`) around the chat
 * column, the mascot stage, and the mascot overlay.
 */
export const ChatMascotProvider = ({ children }: { children: ReactNode }) => {
  const dispatch = useAppDispatch();
  const dockRef = useRef<HTMLElement | null>(null);
  const stageRef = useRef<HTMLElement | null>(null);
  const [dockNode, setDockNodeState] = useState<HTMLElement | null>(null);
  const setDockNode = useCallback((node: HTMLElement | null) => {
    dockRef.current = node;
    setDockNodeState(prev => (prev === node ? prev : node));
  }, []);
  const sendStoreRef = useRef<ChatMascotSendStore | null>(null);
  sendStoreRef.current ??= new ChatMascotSendStore();
  const sendStore = sendStoreRef.current;

  const value = useMemo<ChatMascotContextValue>(
    () => ({
      dockRef,
      setDockNode,
      stageRef,
      sendStore,
      expand: () => {
        mascotLog('[chat-mascot] expand requested');
        dispatch(setChatMascotExpanded(true));
      },
      collapse: () => {
        mascotLog('[chat-mascot] collapse requested');
        dispatch(setChatMascotExpanded(false));
      },
    }),
    [dispatch, sendStore, setDockNode]
  );

  return (
    <ChatMascotContext.Provider value={value}>
      <ChatMascotDockNodeContext.Provider value={dockNode}>
        {children}
      </ChatMascotDockNodeContext.Provider>
    </ChatMascotContext.Provider>
  );
};

/**
 * The mascot handles, or `null` outside the merged chat surface.
 *
 * `Conversations` is also rendered as an embedded sidebar (Flows copilot, iOS)
 * where there is no mascot; those call sites get `null` and skip the dock
 * rather than needing a separate prop.
 */
export function useChatMascotOptional(): ChatMascotContextValue | null {
  return useContext(ChatMascotContext);
}

/** The live dock element, or `null` before it mounts. Overlay-only. */
export function useDockNode(): HTMLElement | null {
  return useContext(ChatMascotDockNodeContext);
}

/** Same, but for components that only ever render inside the provider. */
export function useChatMascot(): ChatMascotContextValue {
  const ctx = useContext(ChatMascotContext);
  if (!ctx) {
    throw new Error('useChatMascot must be used inside <ChatMascotProvider>');
  }
  return ctx;
}

/**
 * Publish the chat's send path to the mascot stage.
 *
 * Called from `Conversations`; a no-op (but still hook-safe) when the mascot
 * surface is absent. The binding is cleared on unmount so a stale `submit`
 * closure can never outlive the chat that owns it.
 */
export function useChatMascotSendBinding(
  ctx: ChatMascotContextValue | null,
  binding: ChatMascotSendBinding
): void {
  const { submit, onError, disabled } = binding;
  const store = ctx?.sendStore;
  useEffect(() => {
    if (!store) return;
    store.set({ submit, onError, disabled });
    return () => store.set(null);
  }, [store, submit, onError, disabled]);
}
