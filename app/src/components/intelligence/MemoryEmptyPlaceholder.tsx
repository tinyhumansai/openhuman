/**
 * Right-pane placeholder shown to brand-new users (zero chunks).
 *
 * Centered, generous whitespace, no call-to-action buttons — the only path
 * forward is connecting an integration in Settings, so we point there in
 * prose without an explicit link to keep the surface meditative.
 */
import { useT } from '../../lib/i18n/I18nContext';

export function MemoryEmptyPlaceholder() {
  const { t } = useT();
  return (
    <div className="mw-detail-empty" data-testid="memory-empty-placeholder">
      <h2 className="mw-empty-title">{t('memory.empty')}</h2>
      <p className="mw-empty-body">{t('memory.emptyHint')}</p>
    </div>
  );
}
