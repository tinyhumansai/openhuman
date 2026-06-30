/**
 * TranscriptViewer — renders a list of transcript lines with speaker labels
 * and timestamps, plus copy-all and download controls.
 */
import debug from 'debug';

import { useT } from '../../lib/i18n/I18nContext';
import { type MeetCallTranscriptLine, parseTranscriptLine } from '../../services/meetCallService';
import Button from '../ui/Button';

const log = debug('meetings:transcript');

interface TranscriptViewerProps {
  lines: MeetCallTranscriptLine[];
}

function buildPlainText(lines: MeetCallTranscriptLine[]): string {
  return lines
    .map(line => {
      const parsed = parseTranscriptLine(line);
      const ts = parsed.timestamp ? `${parsed.timestamp} ` : '';
      const speaker = parsed.speaker ? `${parsed.speaker}: ` : '';
      return `${ts}${speaker}${parsed.text}`;
    })
    .join('\n');
}

export function TranscriptViewer({ lines }: TranscriptViewerProps) {
  const { t } = useT();

  async function handleCopy() {
    log('[transcript] copy all clicked, lines=%d', lines.length);
    try {
      await navigator.clipboard.writeText(buildPlainText(lines));
      log('[transcript] copy succeeded');
    } catch (e) {
      log('[transcript] copy failed', e);
    }
  }

  function handleDownload() {
    log('[transcript] download clicked, lines=%d', lines.length);
    const text = buildPlainText(lines);
    const blob = new Blob([text], { type: 'text/plain' });
    const url = URL.createObjectURL(blob);
    const a = document.createElement('a');
    a.href = url;
    a.download = 'transcript.txt';
    a.click();
    URL.revokeObjectURL(url);
  }

  return (
    <div className="space-y-1">
      <div className="flex items-center justify-between">
        <p className="text-[10px] font-semibold uppercase tracking-wide text-content-muted">
          {t('skills.meetingBots.callTranscriptHeading')}
        </p>
        <div className="flex gap-1">
          <Button variant="tertiary" size="xs" onClick={() => void handleCopy()}>
            {t('skills.meetingBots.history.copyTranscript')}
          </Button>
          <Button variant="tertiary" size="xs" onClick={handleDownload}>
            {t('skills.meetingBots.history.downloadTranscript')}
          </Button>
        </div>
      </div>
      <div className="max-h-64 overflow-y-auto rounded-md bg-surface-muted p-2 space-y-0.5">
        {lines.map((line, i) => {
          const parsed = parseTranscriptLine(line);
          const isAssistant = parsed.role === 'assistant';
          return (
            <p
              key={i}
              className={
                isAssistant
                  ? 'text-[10px] text-primary-600 dark:text-primary-400'
                  : 'text-[10px] text-content-secondary'
              }>
              {parsed.timestamp && (
                <span className="mr-1 text-content-faint">{parsed.timestamp}</span>
              )}
              {parsed.speaker && <span className="mr-1 font-medium">{parsed.speaker}:</span>}
              {parsed.text}
            </p>
          );
        })}
      </div>
    </div>
  );
}

export default TranscriptViewer;
