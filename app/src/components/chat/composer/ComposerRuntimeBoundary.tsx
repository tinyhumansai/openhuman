import { type AssistantState, useAuiState } from '@assistant-ui/react';
import debugFactory from 'debug';
import type { ReactNode } from 'react';

import { DetachedComposerRuntime } from './DetachedComposerRuntime';

const debug = debugFactory('openhuman:chat-composer');

/**
 * Module-level so `useAuiState` keys its cache on selector identity.
 *
 * `s.optional.<scope>` is assistant-ui's documented way to read a scope that
 * may not exist: it resolves to `undefined` instead of throwing, which plain
 * `s.composer` does when no runtime is mounted above. That is what makes this
 * check possible at all — `useAui()` itself succeeds outside a provider and
 * hands back a sentinel client whose scopes throw only when touched.
 */
const selectHasComposer = (s: AssistantState) => s.optional.composer != null;

/**
 * Guarantees `ChatComposer` has an assistant-ui runtime, without letting it
 * borrow the wrong one.
 *
 * The composer is built on `ComposerPrimitive`, which resolves its runtime from
 * React context. That creates a failure mode worth naming: a surface inside the
 * app shell that mounts no runtime of its own does not error — it silently
 * inherits the app-wide one from `ChatRuntimeProvider`, which is bound to
 * `state.thread.selectedThreadId`, the HOME chat's thread. Two surfaces were in
 * in exactly that position: the flows Workflow Copilot, whose own provider
 * closed around its transcript only. It now mounts its own runtime explicitly,
 * because a composer pointed at an unrelated conversation is a data-loss-shaped
 * bug.
 *
 * This boundary is the net under that decision, not a replacement for it. Where
 * a runtime already exists it is a pass-through and the surface keeps whatever
 * scoping it chose. Where none exists — page-level tests that mount a chat
 * subtree directly, and any future host — it supplies an inert one rather than
 * throwing on the first render. The one thing it cannot do is second-guess an
 * *inherited* runtime: from inside the subtree a wrong thread and a right one
 * look identical, which is why the scoping decision has to be made at the mount
 * site.
 *
 * The branch is stable per surface: a subtree either has a provider above it or
 * does not, so this never flips and never remounts the composer mid-life.
 */
export function ComposerRuntimeBoundary({ children }: { children: ReactNode }) {
  const hasComposer = useAuiState(selectHasComposer);
  if (hasComposer) return <>{children}</>;
  debug(
    '[chat-composer] no assistant-ui runtime in context; mounting a detached one for the composer'
  );
  return <DetachedComposerRuntime>{children}</DetachedComposerRuntime>;
}

export default ComposerRuntimeBoundary;
