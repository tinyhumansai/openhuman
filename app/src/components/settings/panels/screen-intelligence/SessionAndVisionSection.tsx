import {
  fetchAccessibilityVisionRecent,
  flushAccessibilityVision,
  startAccessibilitySession,
  stopAccessibilitySession,
} from '../../../../store/accessibilitySlice';
import { useAppDispatch } from '../../../../store/hooks';
import type {
  AccessibilityStatus,
  AccessibilityVisionSummary,
} from '../../../../utils/tauriCommands';

interface SessionAndVisionSectionProps {
  status: AccessibilityStatus | null;
  isStartingSession: boolean;
  isStoppingSession: boolean;
  isFlushingVision: boolean;
  isLoadingVision: boolean;
  startDisabled: boolean;
  stopDisabled: boolean;
  remaining: string;
  screenMonitoring: boolean;
  deviceControl: boolean;
  predictiveInput: boolean;
  recentVisionSummaries: AccessibilityVisionSummary[];
}

const SessionAndVisionSection = ({
  status,
  isStartingSession,
  isStoppingSession,
  isFlushingVision,
  isLoadingVision,
  startDisabled,
  stopDisabled,
  remaining,
  screenMonitoring,
  deviceControl,
  predictiveInput,
  recentVisionSummaries,
}: SessionAndVisionSectionProps) => {
  const dispatch = useAppDispatch();

  return (
    <>
      <section className="space-y-3">
        <h3 className="text-sm font-semibold text-stone-900">Session</h3>
        <div className="text-sm text-stone-600 space-y-1">
          <div>Status: {status?.session.active ? 'Active' : 'Stopped'}</div>
          <div>Remaining: {remaining}</div>
          <div>Frames (ephemeral): {status?.session.frames_in_memory ?? 0}</div>
          <div>Panic stop: {status?.session.panic_hotkey ?? 'Cmd+Shift+.'}</div>
          <div>Vision: {status?.session.vision_state ?? 'idle'}</div>
          <div>Vision queue: {status?.session.vision_queue_depth ?? 0}</div>
          <div>
            Last vision:{' '}
            {status?.session.last_vision_at_ms
              ? new Date(status.session.last_vision_at_ms).toLocaleTimeString()
              : 'n/a'}
          </div>
        </div>

        <div className="flex gap-2">
          <button
            type="button"
            onClick={() =>
              void dispatch(
                startAccessibilitySession({
                  consent: true,
                  ttl_secs: status?.config.session_ttl_secs ?? 300,
                  screen_monitoring: screenMonitoring,
                  device_control: deviceControl,
                  predictive_input: predictiveInput,
                })
              )
            }
            disabled={startDisabled}
            className="rounded-lg border border-green-400 bg-green-50 px-3 py-2 text-sm text-green-700 disabled:opacity-50">
            {isStartingSession ? 'Starting…' : 'Start Session'}
          </button>
          <button
            type="button"
            onClick={() => void dispatch(stopAccessibilitySession('manual_stop'))}
            disabled={stopDisabled}
            className="rounded-lg border border-red-400 bg-red-50 px-3 py-2 text-sm text-red-700 disabled:opacity-50">
            {isStoppingSession ? 'Stopping…' : 'Stop Session'}
          </button>
          <button
            type="button"
            onClick={() => void dispatch(flushAccessibilityVision())}
            disabled={isFlushingVision || !status?.session.active}
            className="rounded-lg border border-primary-400 bg-primary-50 px-3 py-2 text-sm text-primary-700 disabled:opacity-50">
            {isFlushingVision ? 'Analyzing…' : 'Analyze Now'}
          </button>
        </div>
      </section>

      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-sm font-semibold text-stone-900">Vision Summaries</h3>
          <button
            type="button"
            onClick={() => void dispatch(fetchAccessibilityVisionRecent(10))}
            disabled={isLoadingVision}
            className="rounded-lg border border-stone-200 bg-stone-50 px-3 py-1.5 text-xs text-stone-600 disabled:opacity-50">
            {isLoadingVision ? 'Refreshing…' : 'Refresh'}
          </button>
        </div>

        {recentVisionSummaries.length === 0 ? (
          <div className="text-xs text-stone-500">No summaries yet.</div>
        ) : (
          <div className="space-y-2">
            {recentVisionSummaries.map(summary => (
              <div
                key={summary.id}
                className="rounded-xl border border-stone-200 bg-white p-3 text-xs text-stone-200">
                <div className="text-stone-500">
                  {new Date(summary.captured_at_ms).toLocaleTimeString()} ·{' '}
                  {summary.app_name ?? 'Unknown App'}
                  {summary.window_title ? ` · ${summary.window_title}` : ''}
                </div>
                <div className="mt-1 text-stone-800">{summary.actionable_notes}</div>
              </div>
            ))}
          </div>
        )}
      </section>
    </>
  );
};

export default SessionAndVisionSection;
