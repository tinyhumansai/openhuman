import {
  AssistantRuntimeProvider,
  AuiConfig,
  Tools,
  useExternalStoreRuntime,
} from '@assistant-ui/react';
import debugFactory from 'debug';
import { createContext, type ReactNode, useContext, useMemo } from 'react';

import { useOpenHumanToolkit } from '../features/conversations/aui/toolkit';
import { useAppSelector } from '../store/hooks';
import { ComposerDictationContext } from './ComposerDictationContext';
import { useComposerDictation } from './useComposerDictation';
import { useOpenHumanExternalStore } from './useOpenHumanExternalStore';

const debug = debugFactory('openhuman:assistant-ui');

const AuiThreadIdContext = createContext<string | null>(null);

/**
 * The OpenHuman thread this assistant-ui runtime represents.
 *
 * assistant-ui's own context carries its internal thread identity, not ours, so
 * a component rendered *inside* the transcript (a tool part, say) has no other
 * way to name the thread it belongs to. Reading `selectedThreadId` from Redux
 * instead would be wrong on any surface whose thread is not the selected one —
 * the Workflow Copilot mounts a runtime on its own builder thread — which is
 * the same trap {@link AssistantUiRuntimeProvider} documents for messages.
 *
 * `null` outside a runtime, or on a surface that has not created its thread yet.
 */
export function useAuiThreadId(): string | null {
  return useContext(AuiThreadIdContext);
}

/**
 * Mounts assistant-ui's runtime over one OpenHuman thread.
 *
 * Settled process history (reasoning, tools and sub-agents) is read directly
 * from the core transcript RPC, whose Rust projection cache is authoritative.
 * Redux only supplies the existing message list and live socket deltas.
 *
 * ## Why the thread is a prop
 *
 * It used to read `state.thread.selectedThreadId` itself. That is wrong for any
 * surface whose thread is NOT the selected one, and there is exactly such a
 * surface: the Workflow Copilot (`WorkflowCopilotPanel`) renders the shared
 * assistant-ui `Thread` against its own dedicated builder thread, which is
 * never equal to `selectedThreadId`. Reading the selection here would paint
 * the HOME chat's messages inside the copilot. So the thread is chosen by whoever mounts
 * the runtime, and two instances with different thread ids can coexist —
 * assistant-ui's `AssistantRuntimeProvider` is ordinary React context, so the
 * nearest one wins for each subtree.
 *
 * ## The default
 *
 * `threadId` is optional and defaults to `selectedThreadId`, which is what the
 * app-wide mount in `ChatRuntimeProvider` wants: it sits above the home chat and
 * must follow the user's thread selection. `undefined` (prop omitted) means
 * "follow the selection"; an explicit `null` means "this surface has no thread
 * yet" (the copilot before its first send creates one) and is NOT a request to
 * fall back — falling back there is precisely the bug above.
 */
export function AssistantUiRuntimeProvider({
  threadId,
  welcomeSuggestions = true,
  children,
}: {
  /**
   * The thread this runtime instance represents. Omit to follow the globally
   * selected thread; pass `null` for a surface that owns a thread but has not
   * created it yet.
   */
  threadId?: string | null;
  /** Offer the home chat's starter prompts on an empty thread; see the store. */
  welcomeSuggestions?: boolean;
  children: ReactNode;
}) {
  const selectedThreadId = useAppSelector(state => state.thread.selectedThreadId);
  const effectiveThreadId = threadId === undefined ? selectedThreadId : threadId;
  debug(
    '[assistant-ui] runtime scope thread=%s source=%s',
    effectiveThreadId ?? '(none)',
    threadId === undefined ? 'selection' : 'explicit'
  );
  const dictation = useComposerDictation(effectiveThreadId);
  const adapter = useOpenHumanExternalStore(effectiveThreadId, {
    welcomeSuggestions,
    dictationAdapter: dictation.adapter,
  });
  const runtime = useExternalStoreRuntime(adapter);
  // Registers every `aui/toolkit.tsx` entry (currently just `task`) so
  // assistant-ui resolves them ahead of the surface's own `ToolFallback`.
  // Every tool name not in the registry is unaffected: it still renders
  // through `components.ToolFallback` (`ChatToolFallback`) exactly as today.
  const toolkit = useOpenHumanToolkit();
  const config = useMemo(() => AuiConfig({ tools: Tools({ toolkit }) }), [toolkit]);
  return (
    <AssistantRuntimeProvider runtime={runtime} config={config}>
      <AuiThreadIdContext.Provider value={effectiveThreadId}>
        <ComposerDictationContext.Provider value={dictation}>
          {children}
        </ComposerDictationContext.Provider>
      </AuiThreadIdContext.Provider>
    </AssistantRuntimeProvider>
  );
}

export default AssistantUiRuntimeProvider;
