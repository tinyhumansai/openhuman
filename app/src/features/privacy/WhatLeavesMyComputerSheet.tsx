import Button from '../../components/ui/Button';
import {
  DialogContent,
  DialogDescription,
  DialogRoot,
  DialogTitle,
} from '../../components/ui/Dialog';
import { useT } from '../../lib/i18n/I18nContext';
import { WHAT_LEAVES_HEADLINE, WHAT_LEAVES_ITEMS, WHAT_LEAVES_SUBHEAD } from './whatLeavesItems';

interface WhatLeavesMyComputerSheetProps {
  open: boolean;
  onClose: () => void;
}

const WhatLeavesMyComputerSheet = ({ open, onClose }: WhatLeavesMyComputerSheetProps) => {
  const { t } = useT();

  return (
    <DialogRoot
      open={open}
      onOpenChange={next => {
        if (!next) onClose();
      }}>
      <DialogContent
        aria-labelledby="what-leaves-title"
        aria-describedby="what-leaves-description"
        className="mx-4 max-w-lg border border-line p-6"
        overlayClassName="bg-neutral-900/40">
        <div className="flex items-start justify-between gap-4 mb-4">
          <div>
            <DialogTitle asChild>
              <h2 id="what-leaves-title" className="font-title text-2xl text-content leading-tight">
                {WHAT_LEAVES_HEADLINE}
              </h2>
            </DialogTitle>
          </div>
        </div>
        <DialogDescription
          id="what-leaves-description"
          className="text-sm text-content-secondary mb-5 max-w-md">
          {WHAT_LEAVES_SUBHEAD}
        </DialogDescription>

        <ul className="space-y-3 mb-6">
          {WHAT_LEAVES_ITEMS.map(item => (
            <li key={item.id} className="rounded-xl border border-line bg-surface-muted p-4">
              <p className="text-sm font-medium text-content">{item.title}</p>
              <p className="text-sm text-content-secondary mt-1 leading-relaxed">{item.body}</p>
            </li>
          ))}
        </ul>

        <div className="flex items-center justify-between gap-3">
          <p className="text-xs text-content-muted">{t('privacy.description')}</p>
          <Button variant="primary" size="md" onClick={onClose}>
            {t('common.ok')}
          </Button>
        </div>
      </DialogContent>
    </DialogRoot>
  );
};

export default WhatLeavesMyComputerSheet;
