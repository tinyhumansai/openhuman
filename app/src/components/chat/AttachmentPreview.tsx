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
          className="relative flex items-center gap-2 rounded-lg border border-stone-200 dark:border-neutral-700 bg-stone-50 dark:bg-neutral-800 px-2 py-1.5 text-xs text-stone-700 dark:text-neutral-300 max-w-[180px]">
          <img
            src={attachment.dataUri}
            alt={attachment.file.name}
            className="w-8 h-8 rounded object-cover flex-shrink-0"
          />
          <div className="flex flex-col min-w-0">
            <span className="truncate font-medium leading-tight">{attachment.file.name}</span>
            <span className="text-stone-400 dark:text-neutral-500 leading-tight">
              {formatFileSize(attachment.file.size)}
            </span>
          </div>
          <button
            type="button"
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
