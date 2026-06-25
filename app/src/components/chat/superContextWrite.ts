// Tracks the most recent in-flight "super context" config write so the chat
// send path can await it before a new thread's session is built.
//
// The flag is read by the core when the chat task constructs its session, so a
// flip-then-immediately-Send could otherwise race: the fire-and-forget write
// from the composer toggle may still be in flight, and the first turn would run
// with the *previous* persisted value while the UI already shows the new one.
// `SuperContextToggle` registers its write here; `handleSendMessage` awaits
// `whenSuperContextWriteSettled()` before sending.

let pending: Promise<unknown> = Promise.resolve();

/** Register an in-flight super-context write. Failures are swallowed — the
 * send path only needs the write to have *settled*, not to have succeeded. */
export function trackSuperContextWrite(write: Promise<unknown>): void {
  // Chain so concurrent flips all settle before the gate resolves.
  const prior = pending;
  pending = Promise.allSettled([prior, write]);
}

/** Resolves once every registered super-context write has settled. Cheap to
 * await when nothing is pending (resolved promise). */
export function whenSuperContextWriteSettled(): Promise<unknown> {
  return pending;
}
