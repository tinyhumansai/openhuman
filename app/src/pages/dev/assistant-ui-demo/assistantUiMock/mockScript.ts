/**
 * The canned transcript the demo replays.
 *
 * Kept apart from the adapter so the *content* (what a turn contains) and the
 * *timing* (how it arrives) can be read separately. Everything here is fiction:
 * no file is read, no search is run, no subagent exists.
 *
 * The order below is the order the parts are *created*, not the order they
 * finish. A `subagent` step is dispatched and the script moves on immediately —
 * delegation is asynchronous and does not block the turn — so its nested steps
 * land while later tool calls and prose are already streaming. See
 * `mockChatModel` for how that is scheduled.
 */

/**
 * JSON-safe argument payload. Tool-call parts require their `args` to be plain
 * JSON (`ReadonlyJSONObject` upstream); `Record<string, unknown>` is wider than
 * that and does not satisfy it.
 */
type JsonValue = string | number | boolean | null | readonly JsonValue[] | JsonObject;
export type JsonObject = { readonly [key: string]: JsonValue };

export type MockSubagentStep = {
  /** Tool the subagent reached for. */
  tool: string;
  /** One-line, human-readable detail. */
  detail: string;
};

export type MockSubagentResult = {
  subagent: string;
  status: 'running' | 'complete';
  steps: MockSubagentStep[];
  report?: string;
  /**
   * Seconds since dispatch, ticked while the delegation runs. Rendering it is
   * what makes "still going while the answer streams" visible rather than
   * merely true.
   */
  elapsedSeconds?: number;
};

/** A tool call in the script, with the result it eventually returns. */
export type MockToolStep = {
  kind: 'tool';
  toolName: string;
  args: JsonObject;
  /** Milliseconds the call "runs" before its result lands. */
  runMs: number;
  result: unknown;
};

/** A subagent delegation, whose nested steps stream in one at a time. */
export type MockSubagentCall = {
  kind: 'subagent';
  subagent: string;
  args: JsonObject;
  steps: MockSubagentStep[];
  /** Milliseconds between nested steps. */
  stepMs: number;
  report: string;
};

/** Streamed thinking tokens. */
export type MockReasoning = { kind: 'reasoning'; text: string };

/** Streamed assistant prose (markdown). */
export type MockText = { kind: 'text'; text: string };

export type MockStep = MockReasoning | MockToolStep | MockSubagentCall | MockText;

const INTRO = `Looking at this now — I'll hand the deeper reads to a couple of subagents and keep working while they run.`;

const ANSWER = `Here is what this demo is showing you.

This is the upstream [assistant-ui \`base\` example](https://www.assistant-ui.com/demos/base), vendored into the app and driven by a **mock** adapter. Nothing you type leaves the browser — every step above was scripted.

What the transcript exercised, in order:

1. **thinking tokens** — two reasoning blocks, streamed a word at a time and grouped into the chain-of-thought
2. **tool calls** — \`web_search\` and \`read_file\`, each showing streaming arguments before their result lands
3. **subagent calls** — two \`task\` delegations. They are dispatched and *not* awaited: their nested steps and reports landed above while this answer was already streaming, which is what delegation actually looks like
4. **streamed prose** — this answer, including a list and a code block

\`\`\`ts
// the whole turn is a script, not a model
const runtime = useLocalRuntime(mockChatModelAdapter);
\`\`\`

Send another message to replay it.`;

export const MOCK_SCRIPT: readonly MockStep[] = [
  {
    kind: 'reasoning',
    text: `The user is exercising the demo, so there is no real question to answer. What I can do is make the turn cover every part the transcript knows how to render, in the order a real turn would produce them.`,
  },
  { kind: 'text', text: INTRO },

  // Dispatched here, but its four nested steps land during the `web_search`
  // call and the reasoning block below — the turn does not wait for it.
  {
    kind: 'subagent',
    subagent: 'code-explorer',
    args: {
      subagent_type: 'code-explorer',
      description: 'Map the vendored component set',
      prompt: 'List every component under components/assistant-ui and what each one renders.',
    },
    stepMs: 900,
    steps: [
      { tool: 'glob', detail: 'app/src/components/assistant-ui/**/*.tsx — 19 files' },
      { tool: 'read_file', detail: 'thread.tsx — viewport, composer, action bar' },
      { tool: 'read_file', detail: 'tool-group.tsx — collapsible group of tool calls' },
      { tool: 'grep', detail: 'ToolCallMessagePartComponent — 3 matches' },
    ],
    report:
      'Nineteen components. `thread.tsx` owns the viewport and composer; `tool-group.tsx` collapses consecutive tool calls; `reasoning.tsx` renders the thinking block. All of them read shadcn semantic tokens, so they follow the app theme.',
  },

  {
    kind: 'tool',
    toolName: 'web_search',
    args: { query: 'assistant-ui base example thread primitives', max_results: 5 },
    runMs: 1400,
    result: {
      results: [
        { title: 'assistant-ui — base demo', url: 'https://www.assistant-ui.com/demos/base' },
        { title: 'Thread primitives', url: 'https://www.assistant-ui.com/docs/ui/Thread' },
      ],
      took_ms: 812,
    },
  },

  // A second delegation, dispatched while the first is still going.
  {
    kind: 'subagent',
    subagent: 'test-runner',
    args: {
      subagent_type: 'test-runner',
      description: 'Check the demo route typechecks',
      prompt: 'Run the typecheck and report anything that fails in the demo directory.',
    },
    stepMs: 2600,
    steps: [
      { tool: 'shell', detail: 'pnpm typecheck' },
      { tool: 'shell', detail: 'eslint src/pages/dev/assistant-ui-demo' },
    ],
    report: 'Typecheck clean, no lint errors in the demo directory.',
  },

  {
    kind: 'reasoning',
    text: `Both delegations are still working. Nothing about them blocks this turn, so I can keep going and fold their reports in when they land.`,
  },
  {
    kind: 'tool',
    toolName: 'read_file',
    args: { path: 'app/src/pages/dev/assistant-ui-demo/BaseDemo.tsx', offset: 592, limit: 40 },
    runMs: 700,
    result: {
      path: 'app/src/pages/dev/assistant-ui-demo/BaseDemo.tsx',
      lines: 40,
      excerpt: '<MessagePrimitive.GroupedParts groupBy={groupPartByType({ … })}>',
    },
  },
  { kind: 'text', text: ANSWER },
];

/**
 * The turn's closing paragraph, written once the delegations have landed.
 *
 * A delegation that finishes into its own collapsed block is only half of what
 * dispatching means: the point of handing work off is that the answer folds the
 * result back in when it arrives. The main prose streams *before* these finish,
 * so it cannot reference them — this is the part that can, and it is emitted
 * only after the last one reports.
 */
export function buildClosing(reports: readonly { subagent: string; report: string }[]): string {
  if (reports.length === 0) return '';
  const lines = reports.map(r => `- **${r.subagent}** — ${r.report}`).join('\n');
  return `Both delegations have since reported back:\n\n${lines}`;
}

/** Every delegation in the script, in dispatch order, with its report. */
export function scriptedReports(): { subagent: string; report: string }[] {
  return MOCK_SCRIPT.filter((step): step is MockSubagentCall => step.kind === 'subagent').map(
    step => ({ subagent: step.subagent, report: step.report })
  );
}

/** The prompt the seeded transcript is a reply to. */
export const SEED_PROMPT = 'Show me everything this transcript can render.';

/**
 * `MOCK_SCRIPT` as a finished turn, for `initialMessages`.
 *
 * The demo used to open on the empty welcome screen, so opening the page showed
 * nothing until you typed — which is not much of a demo. Seeding the first
 * thread means the reasoning blocks, tool calls and subagent delegations are on
 * screen immediately, and sending a message still replays them streaming.
 *
 * Derived from the same script the adapter streams, so the two cannot drift.
 */
export function buildSeedMessages() {
  const content = MOCK_SCRIPT.map((step, index) => {
    switch (step.kind) {
      case 'reasoning':
        return { type: 'reasoning' as const, text: step.text };
      case 'text':
        return { type: 'text' as const, text: step.text };
      case 'tool':
        return {
          type: 'tool-call' as const,
          toolCallId: `seed-tool-${index}`,
          toolName: step.toolName,
          args: step.args,
          argsText: JSON.stringify(step.args, null, 2),
          result: step.result,
        };
      case 'subagent':
        return {
          type: 'tool-call' as const,
          toolCallId: `seed-task-${index}`,
          toolName: 'task',
          args: step.args,
          argsText: JSON.stringify(step.args, null, 2),
          result: {
            subagent: step.subagent,
            status: 'complete',
            steps: step.steps,
            report: step.report,
            // What a live run of this same step would have taken, so the seeded
            // turn and a replayed one read the same rather than one of them
            // silently dropping the clock.
            elapsedSeconds: Math.round(((step.steps.length + 1) * step.stepMs) / 100) / 10,
          } satisfies MockSubagentResult,
        };
    }
  });

  return [
    { role: 'user' as const, content: [{ type: 'text' as const, text: SEED_PROMPT }] },
    {
      role: 'assistant' as const,
      content: [...content, { type: 'text' as const, text: buildClosing(scriptedReports()) }],
    },
  ];
}
