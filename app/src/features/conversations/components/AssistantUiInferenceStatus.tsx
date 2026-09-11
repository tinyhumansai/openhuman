import { type AssistantState, useAuiState } from '@assistant-ui/react';

import { readOpenHumanThreadExtras } from '../../../providers/useOpenHumanExternalStore';
import { InferenceStatusLine } from './aui/InferenceStatusLine';

const selectThreadExtras = (state: AssistantState) => state.thread.extras;

/**
 * The "what is the model doing right now" line, on the assistant-ui surface.
 *
 * assistant-ui knows only `thread.isRunning`, so before this the whole of
 * `chatRuntime.inferenceStatusByThread` — reasoning round, active tool,
 * delegated sub-agent — stopped at `ChatThreadView`, which `/chat` no longer
 * renders. A slow turn was an unlabelled spinner.
 *
 * The status arrives on the runtime's `extras` channel
 * (`useOpenHumanExternalStore`), so it is always the status of the thread *this
 * runtime* represents; this component holds no Redux read of its own and is
 * therefore safe on a second runtime such as the Workflow Copilot's.
 *
 * Rendering is shared with the legacy surface (`InferenceStatusLine`) so the
 * mic-cloud composer and `/chat` cannot drift apart, and so is the rule for
 * when to show it: the `tool_use` / `subagent` phases only restate the running
 * row, which this surface already paints as a tool part, so the line would be
 * a duplicate caption under the card. It is kept for `thinking` — the phase
 * with no row of its own, and the one a long turn spends minutes in — and as a
 * fallback whenever the phase's row is not on screen (a restored snapshot, or
 * a row that settled ahead of the status), where `status.activeTool` /
 * `status.activeSubagent` is the only name for the work in flight.
 */
export function AssistantUiInferenceStatus() {
  const extras = readOpenHumanThreadExtras(useAuiState(selectThreadExtras));
  const status = extras?.inferenceStatus;
  if (!status) return null;

  const activeRow =
    status.phase === 'subagent'
      ? extras?.activeSubagentEntry
      : status.phase === 'tool_use'
        ? extras?.activeToolEntry
        : undefined;
  if (status.phase !== 'thinking' && activeRow) return null;

  return (
    <InferenceStatusLine
      status={status}
      activeToolEntry={extras?.activeToolEntry}
      activeSubagentEntry={extras?.activeSubagentEntry}
    />
  );
}

export default AssistantUiInferenceStatus;
