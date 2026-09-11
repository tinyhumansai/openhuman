/**
 * A mock `ChatModelAdapter` that replays {@link MOCK_SCRIPT}.
 *
 * Nothing here reaches the OpenHuman core, the backend, or any provider. It
 * exists so the assistant-ui surfaces have something to render while the real
 * seams are reconnected: thinking tokens, tool calls with streaming arguments,
 * subagent delegations with nested steps, and streamed markdown prose.
 *
 * ## Subagents are dispatched, not awaited
 *
 * The interesting part is the scheduling. A delegation is asynchronous and
 * non-blocking: the parent turn hands work off and carries on, and the
 * subagent's steps arrive whenever they arrive — often *after* prose that was
 * written later. An adapter that ran the script strictly in order could never
 * show that, because every part would land in the order it was declared.
 *
 * So a `subagent` step starts a background timeline and the main line moves on
 * immediately. Both mutate the same `parts` array; the main line owns the
 * yielding (an async generator has exactly one producer), and background work
 * only sets a dirty flag. Every wait on the main line is therefore chopped into
 * short slices that flush pending background edits, which is what makes a
 * subagent's steps appear *during* a later tool call rather than after it.
 *
 * The turn still cannot finish before its delegations do, so the tail drains
 * them — but by then the answer is already on screen.
 */
import type {
  ChatModelAdapter,
  ChatModelRunOptions,
  ChatModelRunResult,
  ThreadAssistantMessagePart,
} from '@assistant-ui/react';
import debugFactory from 'debug';

import {
  buildClosing,
  type JsonObject,
  MOCK_SCRIPT,
  type MockSubagentCall,
  type MockSubagentResult,
} from './mockScript';

const debug = debugFactory('openhuman:assistant-ui-demo');

/** Delay between streamed text chunks. Slow enough to see, fast enough not to annoy. */
const CHUNK_MS = 16;
/** Pause before the first chunk, so the "working" indicator actually appears. */
const FIRST_CHUNK_MS = 350;
/** Pause while a tool call's arguments stream in before it starts running. */
const ARGS_MS = 260;
/**
 * How finely a main-line wait is sliced. Each slice is a chance to flush
 * background subagent progress, so this is the resolution at which delegation
 * looks concurrent rather than batched.
 */
const SLICE_MS = 60;

type ToolCallPart = ThreadAssistantMessagePart & { type: 'tool-call' };

const sleep = (ms: number, signal: AbortSignal) =>
  new Promise<void>((resolve, reject) => {
    if (signal.aborted) {
      reject(signal.reason);
      return;
    }
    const timer = setTimeout(() => {
      signal.removeEventListener('abort', onAbort);
      resolve();
    }, ms);
    const onAbort = () => {
      clearTimeout(timer);
      reject(signal.reason);
    };
    signal.addEventListener('abort', onAbort, { once: true });
  });

/** Split into word-ish chunks so the stream looks like a model, not a typewriter. */
const chunk = (text: string): string[] => text.match(/\s*\S+/g) ?? [];

export const mockChatModelAdapter: ChatModelAdapter = {
  async *run(options: ChatModelRunOptions): AsyncGenerator<ChatModelRunResult, void> {
    const { abortSignal } = options;
    const runId = options.unstable_assistantMessageId ?? 'run';

    /** Everything emitted so far. Re-yielded in full on every tick. */
    const parts: ThreadAssistantMessagePart[] = [];
    /** Set by background timelines; cleared when the main line flushes. */
    let dirty = false;
    /** Delegations still in flight. The turn drains these before completing. */
    let pending = 0;
    /** Reports, in the order they came back, for the closing paragraph. */
    const reports: { subagent: string; report: string }[] = [];

    const emit = (): ChatModelRunResult => ({ content: [...parts] });

    /**
     * Wait, in slices, flushing background progress as it appears. Delegating
     * with `yield*` keeps the single-producer rule: this is the main line's own
     * generator, not a second one racing it.
     */
    async function* waitFlushing(ms: number) {
      for (let waited = 0; waited < ms; waited += SLICE_MS) {
        await sleep(Math.min(SLICE_MS, ms - waited), abortSignal);
        if (dirty) {
          dirty = false;
          yield emit();
        }
      }
    }

    /**
     * Start a delegation and return immediately.
     *
     * Live progress rides on the part's **args**, not its result, and that is
     * load-bearing rather than a style choice: assistant-ui derives a tool
     * call's status from whether a result is present, so writing progress into
     * `result` would mark the call complete the instant it started. The tool
     * group would then collapse and stop showing its running state while the
     * delegation was very much still going — the exact opposite of what this is
     * meant to show. Streaming args are the channel for a call in flight;
     * `result` is set once, at the end, and carries the report.
     */
    function dispatchSubagent(step: MockSubagentCall, at: number, index: number) {
      const toolCallId = `${runId}-task-${index}`;
      const state: MockSubagentResult = {
        subagent: step.subagent,
        status: 'running',
        steps: [],
        elapsedSeconds: 0,
      };

      const write = (done: boolean) => {
        const snapshot = { ...state, steps: [...state.steps] };
        const args = done
          ? step.args
          : { ...step.args, progress: snapshot as unknown as JsonObject };
        parts[at] = {
          type: 'tool-call',
          toolCallId,
          toolName: 'task',
          args,
          argsText: JSON.stringify(args, null, 2),
          ...(done ? { result: snapshot } : {}),
        };
        dirty = true;
      };
      write(false);

      pending += 1;
      // `Date.now`, not `performance.now`: the latter is not in this project's
      // ESLint globals, and tenth-of-a-second resolution is all this displays.
      const startedAt = Date.now();
      const elapsed = () => Math.round((Date.now() - startedAt) / 100) / 10;

      void (async () => {
        try {
          for (const nested of step.steps) {
            // Tick the clock while waiting, so a long step reads as work in
            // progress rather than a frozen block.
            for (let waited = 0; waited < step.stepMs; waited += SLICE_MS) {
              await sleep(Math.min(SLICE_MS, step.stepMs - waited), abortSignal);
              state.elapsedSeconds = elapsed();
              write(false);
            }
            state.steps.push(nested);
            write(false);
          }
          state.status = 'complete';
          state.report = step.report;
          state.elapsedSeconds = elapsed();
          write(true);
          reports.push({ subagent: step.subagent, report: step.report });
          debug('[assistant-ui-demo] subagent=%s done in %ss', step.subagent, state.elapsedSeconds);
        } catch {
          // Cancelling the turn aborts its delegations too; the parts they left
          // behind stay as they are, which is the honest record of a stopped run.
        } finally {
          pending -= 1;
          dirty = true;
        }
      })();
    }

    debug('[assistant-ui-demo] run start messages=%d', options.messages.length);
    yield* waitFlushing(FIRST_CHUNK_MS);

    for (const [index, step] of MOCK_SCRIPT.entries()) {
      switch (step.kind) {
        case 'reasoning':
        case 'text': {
          const at = parts.length;
          let text = '';
          for (const piece of chunk(step.text)) {
            text += piece;
            parts[at] = { type: step.kind, text };
            dirty = false;
            yield emit();
            await sleep(CHUNK_MS, abortSignal);
          }
          break;
        }

        case 'tool': {
          const at = parts.length;
          const argsText = JSON.stringify(step.args, null, 2);

          // Arguments first, with no result — the "running" state the tool
          // group renders a spinner for.
          parts[at] = {
            type: 'tool-call',
            toolCallId: `${runId}-tool-${index}`,
            toolName: step.toolName,
            args: step.args,
            argsText,
          };
          yield emit();
          yield* waitFlushing(ARGS_MS + step.runMs);

          parts[at] = { ...(parts[at] as ToolCallPart), result: step.result };
          yield emit();
          break;
        }

        case 'subagent': {
          // Reserve the slot, hand the work off, keep going.
          const at = parts.length;
          parts[at] = { type: 'text', text: '' };
          dispatchSubagent(step, at, index);
          yield emit();
          break;
        }
      }
    }

    // The answer is on screen; drain whatever is still delegated.
    while (pending > 0) {
      yield* waitFlushing(SLICE_MS);
      if (dirty) {
        dirty = false;
        yield emit();
      }
    }

    // Now that they have all reported, fold the results back into the answer.
    // This is the half that dispatching would otherwise lose: the prose above
    // streamed before the delegations finished, so it could not cite them.
    const closing = buildClosing(reports);
    if (closing) {
      const at = parts.length;
      let text = '';
      for (const piece of chunk(closing)) {
        text += piece;
        parts[at] = { type: 'text', text };
        yield emit();
        await sleep(CHUNK_MS, abortSignal);
      }
    }

    debug('[assistant-ui-demo] run complete parts=%d reports=%d', parts.length, reports.length);
    yield { content: [...parts], status: { type: 'complete', reason: 'stop' } };
  },
};

export default mockChatModelAdapter;
