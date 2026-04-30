import type { ToolTimelineEntry } from '../../../store/chatRuntimeSlice';
import { formatTimelineEntry } from '../../../utils/toolTimelineFormatting';
import { parseWorkerThreadRef } from '../utils/workerThreadRef';
import { WorkerThreadRefCard } from './WorkerThreadRefCard';

export function ToolTimelineBlock({ entries }: { entries: ToolTimelineEntry[] }) {
  const latestRunningEntryId = [...entries].reverse().find(entry => entry.status === 'running')?.id;

  const normalizeToolBody = (value?: string): string | undefined => {
    if (!value) return undefined;
    const trimmed = value.trim();
    if (trimmed.length === 0) return undefined;
    if (trimmed === '{}' || trimmed === '[]' || trimmed === 'null') return undefined;
    return value;
  };

  return (
    <div className="mb-2 space-y-1 px-1 py-0">
      {entries.map(entry => {
        const formatted = formatTimelineEntry(entry);
        const detailContent =
          normalizeToolBody(formatted.detail) ?? normalizeToolBody(entry.argsBuffer);
        const workerRef = parseWorkerThreadRef(formatted.detail ?? entry.detail);
        const shouldAutoExpand = latestRunningEntryId != null && latestRunningEntryId === entry.id;
        const statusTone =
          entry.status === 'running'
            ? {
                pill: 'bg-amber-100 text-amber-600',
                bubble: 'bg-amber-50 text-amber-900',
                code: 'text-amber-800',
                chevron: 'text-amber-500',
              }
            : entry.status === 'success'
              ? {
                  pill: 'bg-sage-100 text-sage-600',
                  bubble: 'bg-sage-50 text-sage-900',
                  code: 'text-sage-800',
                  chevron: 'text-sage-500',
                }
              : {
                  pill: 'bg-coral-100 text-coral-600',
                  bubble: 'bg-coral-50 text-coral-900',
                  code: 'text-coral-800',
                  chevron: 'text-coral-500',
                };

        return (
          <div key={entry.id} className="flex flex-col gap-1 text-xs text-stone-400">
            {detailContent ? (
              <details open={shouldAutoExpand} className="ml-1 group">
                <summary className="flex cursor-pointer list-none items-center gap-2 select-none marker:hidden">
                  <span
                    className={`text-[10px] transition-transform group-open:rotate-90 ${statusTone.chevron}`}>
                    ▶
                  </span>
                  <span className="font-medium text-stone-600">{formatted.title}</span>
                  <span className={`rounded-full px-2 py-0.5 text-[10px] ${statusTone.pill}`}>
                    {entry.status}
                  </span>
                </summary>
                {workerRef ? (
                  <div
                    className={`mt-1 rounded-xl rounded-tl-md px-2.5 py-2 text-[11px] whitespace-pre-wrap break-words ${statusTone.bubble}`}>
                    {workerRef.before}
                    <WorkerThreadRefCard ref={workerRef.ref} />
                    {workerRef.after ? <div className="mt-1">{workerRef.after}</div> : null}
                  </div>
                ) : formatted.detail ? (
                  <div
                    className={`mt-1 rounded-xl rounded-tl-md px-2.5 py-2 text-[11px] whitespace-pre-wrap break-words ${statusTone.bubble}`}>
                    {formatted.detail}
                  </div>
                ) : (
                  <pre
                    className={`mt-1 max-h-24 overflow-y-auto rounded px-2 py-1 font-mono text-[10px] whitespace-pre-wrap break-all ${statusTone.bubble} ${statusTone.code}`}>
                    {detailContent}
                  </pre>
                )}
              </details>
            ) : (
              <div className="ml-1 flex items-center gap-2">
                <span className="font-medium text-stone-600">{formatted.title}</span>
                <span className={`rounded-full px-2 py-0.5 text-[10px] ${statusTone.pill}`}>
                  {entry.status}
                </span>
              </div>
            )}
          </div>
        );
      })}
    </div>
  );
}
