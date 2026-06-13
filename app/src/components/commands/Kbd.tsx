import { memo, useMemo } from 'react';

import { formatShortcut, isMac, parseShortcut } from '../../lib/commands/shortcut';
import { useT } from '../../lib/i18n/I18nContext';

interface Props {
  shortcut: string;
  size?: 'sm' | 'md';
  className?: string;
}

function Kbd({ shortcut, size = 'sm', className = '' }: Props) {
  const { t } = useT();
  const segments = useMemo(() => formatShortcut(parseShortcut(shortcut), isMac()), [shortcut]);
  const padding = size === 'md' ? 'px-2 py-1 text-sm' : 'px-1.5 py-0.5 text-xs';
  return (
    <span
      className={`inline-flex items-center gap-1 font-mono ${className}`}
      aria-label={t('kbd.ariaLabel').replace('{shortcut}', segments.join(' '))}>
      {segments.map((seg, i) => (
        <kbd
          key={i}
          className={`${padding} rounded border border-cmd-border bg-cmd-surface-elevated text-cmd-foreground-muted`}>
          {seg}
        </kbd>
      ))}
    </span>
  );
}

export default memo(Kbd);
