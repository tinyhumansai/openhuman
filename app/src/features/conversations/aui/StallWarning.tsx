/**
 * The "this turn has gone quiet" banner above the composer (after OpenClaw's
 * health chips): what the quiet turn is doing, how long it has been quiet, and
 * Stop right beside it so the user does not have to hunt for the composer's
 * stop button. Informational only — the turn keeps running and the banner
 * clears itself on the next inference signal.
 *
 * Three states, so a healthy long step never reads as an error:
 * - a tool is still running (a build, a CI watch) → neutral, names the tool;
 * - the model is thinking → neutral "still thinking";
 * - nothing is running and no output arrived → amber "may be stuck".
 */
import { Alert, AlertDescription } from '../../../components/ui/Alert';
import Button from '../../../components/ui/Button';
import { useT } from '../../../lib/i18n/I18nContext';
import type { ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import { presentTimelineEntry } from '../../../utils/toolTimelineFormatting';
import { formatElapsed } from '../components/AssistantUiToolCall';
import { toolLabel } from '../tools/toolPresentation';
import { useLiveElapsed } from './useLiveElapsed';

export interface StallWarningProps {
  /** `inferenceStatusByThread.phase` of the quiet turn. */
  phase: string;
  /** Epoch ms of the turn's last inference signal; unknown hides the duration. */
  quietSince: number | undefined;
  /** The turn's tool call still in `running` state, if any. */
  runningTool?: ToolTimelineEntry;
  onStop: () => void;
}

/** `75_000` → "1m 15s", rounded to whole seconds for a banner. */
function formatQuiet(ms: number): string {
  return formatElapsed(Math.round(ms / 1000) * 1000);
}

export function StallWarning({ phase, quietSince, runningTool, onStop }: StallWarningProps) {
  const { t } = useT();
  const quietMs = useLiveElapsed(quietSince, true);
  const duration = quietMs !== undefined ? formatQuiet(quietMs) : undefined;
  // ponytail: an unknown stall moment reads as the 2-min silence window.
  const shown = duration ?? '2m';
  const state = runningTool ? 'tool' : phase === 'thinking' ? 'thinking' : 'stuck';

  let message: string;
  if (runningTool) {
    // Same label as the timeline row. The duration is shown as quiet time
    // below, not as the tool's runtime: a parallel call or sub-agent event
    // mid-run restarts the quiet clock while this tool keeps going.
    const label = toolLabel({ ...presentTimelineEntry(runningTool), tense: 'active' }, t);
    message = t('chat.stallWarning.toolRunning').replace('{tool}', label);
  } else if (state === 'thinking') {
    message = t('chat.stallWarning.thinking');
  } else {
    message = t('chat.stallWarning.working').replace('{duration}', shown);
  }

  return (
    <Alert
      variant={state === 'stuck' ? 'warning' : 'default'}
      density="compact"
      role="status"
      className="mb-2 items-start justify-between">
      <AlertDescription
        data-testid="chat-stall-warning"
        data-chat-stall-phase={phase}
        data-chat-stall-state={state}>
        {message}
        {state !== 'stuck' && duration !== undefined && (
          <>
            {' '}
            <span data-testid="chat-stall-quiet" className="tabular-nums font-medium">
              {t('chat.status.quietFor').replace('{duration}', duration)}
            </span>
          </>
        )}
      </AlertDescription>
      <Button
        variant={state === 'stuck' ? 'secondary' : 'tertiary'}
        size="xs"
        analyticsId="chat-stall-stop"
        data-testid="chat-stall-stop"
        onClick={onStop}
        className="flex-none">
        {t('chat.status.stop')}
      </Button>
    </Alert>
  );
}

export default StallWarning;
