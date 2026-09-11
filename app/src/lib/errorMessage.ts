/**
 * Turn an unknown thrown/rejected value into something worth showing a user.
 *
 * The case this exists for: `dispatch(someThunk()).unwrap()` does NOT reject
 * with an `Error`. For a `createAsyncThunk` that threw, Redux Toolkit rethrows
 * its `SerializedError` — a plain object `{ name, message, stack }` — and for
 * one that used `rejectWithValue`, it rethrows the payload, often a bare
 * string. Neither is an `instanceof Error`.
 *
 * So the common guard
 *
 *   err instanceof Error ? err.message : String(err)
 *
 * takes its `String(err)` branch on exactly the shape it was written to
 * handle, and `String({ message: '…' })` is `"[object Object]"`. The user loses
 * the only diagnostic the failure path gives them (#5900), and the same shape
 * caused the "Non-Error promise rejection captured with value" reports in
 * #5156.
 *
 * Read `message` off the value instead of testing its prototype.
 *
 * Two existing helpers already solve this for their own domains —
 * `formatThreadCreateError` (`store/threadSlice.ts`) and `formatThreadLoadError`
 * (`features/conversations/Conversations.tsx`). Both predate this and carry
 * domain-specific fallbacks and, in the thread case, an extra empty-message
 * rule that #5156 depends on; they are deliberately left alone. New call sites
 * should use this.
 *
 * @param error    the caught value, of unknown shape
 * @param fallback shown when nothing usable can be recovered
 */
export function errorMessage(error: unknown, fallback: string): string {
  if (typeof error === 'string' && error.trim().length > 0) return error.trim();

  // An `Error` with an empty message is no more useful than no error at all —
  // fall through to the fallback rather than rendering a blank alert.
  if (error instanceof Error && error.message.trim().length > 0) {
    return error.message.trim();
  }

  // The SerializedError / rejectWithValue-object case: duck-type `message`
  // rather than testing the prototype, which is the whole bug.
  if (error && typeof error === 'object' && 'message' in error) {
    const message = (error as { message?: unknown }).message;
    if (typeof message === 'string' && message.trim().length > 0) {
      return message.trim();
    }
  }

  return fallback;
}
