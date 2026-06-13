import type { ReactNode } from 'react';

import { useT } from '../../lib/i18n/I18nContext';

interface BetaBannerProps {
  /** Override the default "This feature is in beta" message. */
  children?: ReactNode;
  className?: string;
}

export default function BetaBanner({ children, className }: BetaBannerProps) {
  const { t } = useT();
  return (
    <div
      className={`flex items-start gap-2.5 rounded-lg border border-amber-200 dark:border-amber-500/30 bg-amber-50 dark:bg-amber-500/10 px-3 py-2 text-xs leading-relaxed text-amber-800 dark:text-amber-300 ${className ?? ''}`}
      role="status">
      <span className="mt-px shrink-0 rounded bg-amber-200 dark:bg-amber-500/30 px-1.5 py-0.5 text-[10px] font-bold uppercase tracking-wider text-amber-700 dark:text-amber-300">
        {t('common.beta')}
      </span>
      <span>{children ?? t('common.betaDisclaimer')}</span>
    </div>
  );
}
