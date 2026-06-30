import { type Attachment, formatFileSize } from '../../lib/attachments';
import { useT } from '../../lib/i18n/I18nContext';

interface AttachmentPreviewProps {
  attachments: Attachment[];
  onRemove: (id: string) => void;
  disabled?: boolean;
}

export default function AttachmentPreview({
  attachments,
  onRemove,
  disabled,
}: AttachmentPreviewProps) {
  const { t } = useT();

  if (attachments.length === 0) return null;

  return (
    <div className="flex flex-wrap gap-2 px-1 pb-1">
      {attachments.map(attachment => (
        <div
          key={attachment.id}
          className="relative flex items-center gap-2 rounded-lg border border-line bg-surface-muted px-2 py-1.5 text-xs text-content-secondary max-w-[180px]">
          {attachment.kind === 'image' ? (
            <img
              src={attachment.previewUri ?? attachment.dataUri}
              alt={attachment.file.name}
              className="w-8 h-8 rounded object-cover flex-shrink-0"
            />
          ) : (
            <div
              aria-hidden
              className="w-8 h-8 rounded border border-line bg-surface flex items-center justify-center flex-shrink-0 text-content-muted">
              <svg className="w-4 h-4" fill="none" stroke="currentColor" viewBox="0 0 24 24">
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.8}
                  d="M14 2H6a2 2 0 00-2 2v16a2 2 0 002 2h12a2 2 0 002-2V8z"
                />
                <path
                  strokeLinecap="round"
                  strokeLinejoin="round"
                  strokeWidth={1.8}
                  d="M14 2v6h6M8 13h8M8 17h5"
                />
              </svg>
            </div>
          )}
          <div className="flex flex-col min-w-0">
            <span className="truncate font-medium leading-tight">{attachment.file.name}</span>
            <span className="text-content-faint leading-tight">
              {formatFileSize(attachment.payloadSizeBytes)}
            </span>
          </div>
          <button
            type="button"
            data-analytics-id="chat-attachment-remove"
            aria-label={t('chat.attachment.remove').replace('{name}', attachment.file.name)}
            onClick={() => onRemove(attachment.id)}
            disabled={disabled}
            className="absolute -top-1.5 -right-1.5 w-4 h-4 flex items-center justify-center rounded-full bg-stone-400 dark:bg-neutral-600 text-white hover:bg-stone-600 dark:hover:bg-neutral-400 transition-colors disabled:opacity-40 disabled:cursor-not-allowed flex-shrink-0">
            <svg className="w-2.5 h-2.5" fill="none" stroke="currentColor" viewBox="0 0 24 24">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={3}
                d="M6 18L18 6M6 6l12 12"
              />
            </svg>
          </button>
        </div>
      ))}
    </div>
  );
}
