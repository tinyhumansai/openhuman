/**
 * ToolActivityChip — replaces raw tool-call JSON in the copilot chat with a
 * compact, human-readable status pill (B25). Users should know the agent
 * used a tool (it explains why the turn took longer), but they should never
 * see the raw JSON arguments (e.g. the whole workflow graph payload).
 */
import { useT } from '../../lib/i18n/I18nContext';

interface Props {
  /** Tool names extracted from the turn's tool-call envelope, in call order. */
  toolNames: string[];
}

/** Tools that map to a specific, more informative status label. */
const KNOWN_TOOL_LABEL_KEYS: Record<string, string> = {
  propose_workflow: 'flows.copilot.tool.proposing',
  revise_workflow: 'flows.copilot.tool.proposing',
  dry_run_workflow: 'flows.copilot.tool.dryRunning',
  save_workflow: 'flows.copilot.tool.saving',
};

export default function ToolActivityChip({ toolNames }: Props) {
  const { t } = useT();
  if (toolNames.length === 0) return null;

  // Every tool must map to the SAME recognized label before we show a
  // specific status (e.g. all of `propose_workflow`/`revise_workflow` map to
  // "proposing..."); any unrecognized tool, or a mix of tools with different
  // labels, falls back to a generic "Using tools..." pill rather than
  // picking one tool's label arbitrarily or dumping tool names verbatim.
  const labelKeys = toolNames.map(name => KNOWN_TOOL_LABEL_KEYS[name]);
  const labelKey = labelKeys.every(key => key && key === labelKeys[0]) ? labelKeys[0] : undefined;

  return (
    <span
      data-testid="tool-activity-chip"
      className="mt-1 inline-flex w-fit items-center gap-1 rounded-full bg-surface-subtle px-1.5 py-0.5 text-[11px] font-medium text-content-muted">
      {t(labelKey ?? 'flows.copilot.tool.usingTools')}
    </span>
  );
}
