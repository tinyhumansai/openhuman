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
import type { CoreCommand } from '../../../../features/conversations/aui/useSlashCommandSource';
import type { ContextBreakdown } from '../../../../services/api/agentContextApi';
import type { FetchPage } from '../../../../services/api/memoryApi';
import type { ChatSuggestionsEvent } from '../../../../services/chatService';

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
    text: `**Reading the request**

The user is exercising the demo, so there is no real question to answer.

**Planning the turn**

What I can do is make the turn cover every part the transcript knows how to render, in the order a real turn would produce them: reasoning, tools, delegations, then prose.`,
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
    text: `**Checking on the delegations**

Both delegations are still working. Nothing about them blocks this turn, so I can keep going and fold their reports in when they land.`,
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
  // Exercises the `media_generate_image` toolkit entry
  // (`elements-image-generation` while running, then the `image` element).
  {
    kind: 'tool',
    toolName: 'media_generate_image',
    args: { prompt: 'a minimalist line-art fox reading a book' },
    runMs: 1600,
    result: {
      artifacts: [
        {
          type: 'image',
          source_url: 'https://picsum.photos/seed/openhuman-demo/512',
          artifact_id: 'demo-image-1',
        },
      ],
    },
  },

  // Exercises the `generate_document` toolkit entry (`elements-artifact-card`).
  {
    kind: 'tool',
    toolName: 'generate_document',
    args: { title: 'Demo transcript summary', sections: ['Overview', 'Findings'] },
    runMs: 1200,
    result: { title: 'Demo transcript summary', path: 'artifacts/demo-transcript-summary.docx' },
  },

  // Exercises the `goal_set` toolkit entry (`GoalToolLine.tsx` — a one-line
  // inline summary, distinct from the pinned `AgentStatus` pill above the
  // composer, which is driven live by `thread_goal_updated` instead).
  {
    kind: 'tool',
    toolName: 'goal_set',
    args: { objective: 'Cover every element the demo transcript can render' },
    runMs: 400,
    result: {
      goal: {
        goal_id: 'demo-goal-1',
        objective: 'Cover every element the demo transcript can render',
        status: 'active',
        tokens_used: 1200,
        token_budget: 20000,
        time_used_seconds: 8,
      },
    },
  },

  // Exercises the `todo` toolkit entry (`TodoListPart.tsx` — the vendored
  // `TodoList` element, mapping core `pending|in_progress|completed` onto
  // the element's `pending|active|done|failed`).
  {
    kind: 'tool',
    toolName: 'todo',
    args: {
      todos: [
        { content: 'Stream reasoning and prose', status: 'completed' },
        { content: 'Run a tool call and a delegation', status: 'completed' },
        { content: 'Render the goal and plan-review elements', status: 'in_progress' },
        { content: 'Wrap up with the closing summary', status: 'pending' },
      ],
    },
    runMs: 400,
    result: {
      todos: [
        { content: 'Stream reasoning and prose', status: 'completed' },
        { content: 'Run a tool call and a delegation', status: 'completed' },
        { content: 'Render the goal and plan-review elements', status: 'in_progress' },
        { content: 'Wrap up with the closing summary', status: 'pending' },
      ],
    },
  },

  // Exercises the `request_plan_review` toolkit entry (`PlanReviewPart.tsx` —
  // the vendored `AgentPlan` element). Rendered as already-decided history
  // here (no `pendingPlanReviewByThread` entry backs a seeded/scripted
  // call), so it shows fully "done" rather than the live approve/reject/
  // revise row — see `/dev/tools` for the interactive decision states.
  {
    kind: 'tool',
    toolName: 'request_plan_review',
    args: {
      steps: [
        'Render the goal and todo elements inline',
        'Show the plan under review',
        'Resolve the review and continue',
      ],
    },
    runMs: 400,
    result: {
      steps: [
        'Render the goal and todo elements inline',
        'Show the plan under review',
        'Resolve the review and continue',
      ],
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

/**
 * A running turn with follow-ups queued behind it, for the message-queue
 * element in the dev gallery (`/dev/tools`). Shaped like the core's run queue
 * (`queue_item_queued` → `{ id, text_preview }`) projected to element props.
 */
export const MOCK_MESSAGE_QUEUE = {
  running: SEED_PROMPT,
  queued: [
    { id: 'mock-queue-1', text: 'Then compare it with the legacy composer.' },
    { id: 'mock-queue-2', text: 'And list anything that still renders a custom card.' },
  ],
} as const;

/**
 * A `openhuman.commands_list` response for the composer's `/` picker, shaped
 * like the core catalog (`{ id, label, description?, kind, insert? }`). The
 * gallery (`/dev/tools`) renders it through the vendored composer menu.
 */
export const MOCK_COMMANDS_LIST: CoreCommand[] = [
  { id: 'plan', label: 'Plan', description: 'Plan first', kind: 'builtin' },
  {
    id: 'summarize',
    label: 'Summarize',
    description: 'Summarize this thread',
    kind: 'skill',
    insert: '/summarize ',
  },
  { id: 'weekly-report', label: 'Weekly report', kind: 'workflow' },
];

/**
 * An `openhuman.memory_fetch` response for the composer's `@` picker
 * (Memory category), plus the thread files it lists beside it.
 */
export const MOCK_MEMORY_FETCH: FetchPage = {
  hits: [
    {
      id: 'mock-hit-1',
      kind: 'conversation',
      text: 'Quarterly planning notes: ship the composer pickers first',
      meta: { thread_id: 'mock-thread-1', source: { kind: 'conversation' } },
      score: 0.91,
    },
    {
      id: 'mock-hit-2',
      kind: 'document',
      text: 'Roadmap review with design',
      meta: { file_path: 'roadmap.md', source: { kind: 'folder' } },
      score: 0.74,
    },
  ],
};

export const MOCK_THREAD_FILES = [
  {
    id: 'mock-artifact-1',
    label: 'Signed contract',
    description: 'artifacts/signed-contract.docx',
  },
] as const;

/**
 * The composer's context-usage ring for a thread mid-conversation: the last
 * turn's orchestrator tokens (what `chat_done.usage` leaves in
 * `usageByThread`) against the model's window.
 */
export const MOCK_CONTEXT_USAGE = {
  modelContextWindow: 200_000,
  usage: { totalTokens: 61_400, inputTokens: 58_200, outputTokens: 3_200 },
} as const;

/**
 * An `openhuman.agent_context_breakdown` response for the same thread, as the
 * core shapes it: one row per rendered prompt heading, one `tools` row and one
 * `history` row. The breakdown popover renders it through the vendored
 * context-breakdown element.
 */
export const MOCK_CONTEXT_BREAKDOWN: ContextBreakdown = {
  sections: [
    { label: '(preamble)', bytes: 2_400, est_tokens: 600 },
    { label: '## Identity', bytes: 3_200, est_tokens: 800 },
    { label: '## Tools and delegation', bytes: 9_600, est_tokens: 2_400 },
    { label: '## Memory', bytes: 4_800, est_tokens: 1_200 },
    { label: 'tools', bytes: 72_000, est_tokens: 18_000 },
    { label: 'history', bytes: 154_000, est_tokens: 38_500 },
  ],
  total_est_tokens: 61_500,
  context_window: 200_000,
};

/**
 * Every phase of the thread's connection banner (`ConnectionStateBanner`), for
 * the gallery's phase toggle (`/dev/tools`). In the app the phase follows the
 * renderer's socket status; here it is picked by hand.
 */
export const MOCK_CONNECTION_PHASES = ['dropped', 'reconnecting', 'resumed', 'online'] as const;

/**
 * A settled turn and the `chat_suggestions` event the core emits after its
 * `chat_done` (`web_chat/suggestions.rs`: up to three `{ prompt, label }`
 * pairs). The gallery (`/dev/tools`) runs the event through the same reducer
 * and chip mapping as the app, then renders the vendored follow-up element.
 */
export const MOCK_SUGGESTIONS_TURN = {
  user: 'What is on my calendar today?',
  assistant: 'Two meetings: design review at 11:00 and a 1:1 with Sam at 15:30.',
} as const;

export const MOCK_CHAT_SUGGESTIONS_EVENT: ChatSuggestionsEvent = {
  thread_id: 'mock-suggestions-thread',
  client_id: 'mock-client',
  turn_request_id: 'mock-request-1',
  suggestions: [
    { prompt: 'Move the design review to tomorrow morning', label: 'Reschedule review' },
    { prompt: 'Draft an agenda for my 1:1 with Sam', label: 'Draft 1:1 agenda' },
    { prompt: 'Is anything due before the design review?' },
  ],
};
