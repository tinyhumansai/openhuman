/**
 * Mounts a fully mocked assistant-ui runtime for the vendored `base` demo.
 *
 * Deliberately isolated from the app's real runtime
 * (`providers/AssistantUiRuntimeProvider`, which projects OpenHuman's Redux
 * state through an external store). This one owns its own in-memory state: a
 * thread list that lives for the lifetime of the page, a canned chat model, and
 * attachment/feedback adapters that never leave the browser. Mounting it cannot
 * touch a thread, a message, or the core.
 */
import {
  AssistantRuntimeProvider,
  CompositeAttachmentAdapter,
  InMemoryThreadListAdapter,
  SimpleImageAttachmentAdapter,
  SimpleTextAttachmentAdapter,
  useLocalRuntime,
  useRemoteThreadListRuntime,
} from '@assistant-ui/react';
import debugFactory from 'debug';
import { createContext, type ReactNode, useContext, useId, useMemo, useRef, useState } from 'react';

import { buildSeedMessages, mockChatModelAdapter } from './assistantUiMock';

const debug = debugFactory('openhuman:assistant-ui-demo');

/**
 * Which thread gets the seeded transcript.
 *
 * `runtimeHook` is called once per live thread and takes no arguments, so the
 * "is this the first thread?" answer has to arrive through context. It is
 * keyed on `useId()` rather than a counter because `useId` is stable per hook
 * instance — a counter would be consumed twice under StrictMode's double
 * render and seed the wrong thread.
 */
const FirstThreadContext = createContext<{ id: string | null }>({ id: null });

function useDemoThreadRuntime() {
  const box = useContext(FirstThreadContext);
  const id = useId();
  // The provider owns this stable, page-scoped marker. It is initialized once
  // as the remote thread-list runtime mounts its first child runtime.
  // eslint-disable-next-line react-hooks/immutability
  if (box.id === null) box.id = id;
  const isFirstThread = box.id === id;

  const adapters = useMemo(
    () => ({
      attachments: new CompositeAttachmentAdapter([
        new SimpleImageAttachmentAdapter(),
        new SimpleTextAttachmentAdapter(),
      ]),
      feedback: {
        submit: ({ type }: { type: 'positive' | 'negative' }) => {
          debug('[assistant-ui-demo] feedback=%s (mock, discarded)', type);
        },
      },
    }),
    []
  );

  // Only the thread the page opens on is seeded. A thread the user creates from
  // the rail should start empty, the way a new chat does.
  const initialMessages = useMemo(
    () => (isFirstThread ? buildSeedMessages() : undefined),
    [isFirstThread]
  );

  debug('[assistant-ui-demo] thread runtime seeded=%s', isFirstThread);
  return useLocalRuntime(mockChatModelAdapter, { adapters, initialMessages });
}

export function MockRuntimeProvider({ children }: { children: ReactNode }) {
  const firstThread = useRef<{ id: string | null }>({ id: null });

  // The context has to sit ABOVE the component that calls
  // `useRemoteThreadListRuntime`: that hook renders the per-thread
  // `runtimeHook` inside its own subtree, which a provider returned from this
  // same component body would not cover.
  return (
    <FirstThreadContext.Provider value={firstThread.current}>
      <MockRuntime>{children}</MockRuntime>
    </FirstThreadContext.Provider>
  );
}

function MockRuntime({ children }: { children: ReactNode }) {
  // One adapter instance for the page's lifetime — the options doc requires a
  // stable reference, since replacing it reloads the list and drops threads.
  const [adapter] = useState(() => new InMemoryThreadListAdapter());
  const runtime = useRemoteThreadListRuntime({ runtimeHook: useDemoThreadRuntime, adapter });

  return <AssistantRuntimeProvider runtime={runtime}>{children}</AssistantRuntimeProvider>;
}

export default MockRuntimeProvider;
