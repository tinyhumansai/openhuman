/*
 * Sticky "you have unsaved changes" bar shown at the bottom of the panel.
 */
import { LuCheck, LuCircleAlert } from 'react-icons/lu';

import { useT } from '../../../../lib/i18n/I18nContext';
import Button from '../../../ui/Button';

export const SaveBar = ({
  diffSummary,
  changeCount,
  onSave,
  onDiscard,
}: {
  diffSummary: string[];
  changeCount: number;
  onSave: () => void;
  onDiscard: () => void;
}) => {
  const { t } = useT();
  return (
    <div className="pointer-events-none sticky bottom-3 z-20 flex justify-center px-4">
      <div className="pointer-events-auto flex w-full items-center gap-2 rounded-lg border border-line bg-surface/95 px-3 py-2 shadow-float backdrop-blur-md animate-fade-up">
        <div className="flex h-6 w-6 shrink-0 items-center justify-center rounded bg-amber-50 dark:bg-amber-500/10 text-amber-600 dark:text-amber-300">
          <LuCircleAlert className="h-3.5 w-3.5" />
        </div>
        <div className="min-w-0 flex-1">
          <div className="text-xs font-medium text-content">
            {changeCount === 1
              ? t('settings.ai.unsavedChange')
              : `${String(changeCount)} ${t('settings.ai.unsavedChanges')}`}
          </div>
          <div className="truncate font-mono text-[10px] text-content-muted">
            {diffSummary.slice(0, 2).join(' · ')}
            {diffSummary.length > 2 ? ` · +${diffSummary.length - 2}` : ''}
          </div>
        </div>
        <Button variant="secondary" size="xs" onClick={onDiscard}>
          {t('settings.ai.discard')}
        </Button>
        <Button
          variant="primary"
          size="xs"
          onClick={onSave}
          leadingIcon={<LuCheck className="h-3 w-3" />}>
          {t('common.save')}
        </Button>
      </div>
    </div>
  );
};

export default SaveBar;
