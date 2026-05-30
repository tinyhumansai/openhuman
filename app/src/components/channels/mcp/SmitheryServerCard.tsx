/**
 * Card component for a single Smithery registry server.
 * Shows icon, name, description (clamped), usage count and deployed badge.
 */
import { useT } from '../../../lib/i18n/I18nContext';
import type { SmitheryServer } from './types';

interface SmitheryServerCardProps {
  server: SmitheryServer;
  onInstall: (qualifiedName: string) => void;
}

const SmitheryServerCard = ({ server, onInstall }: SmitheryServerCardProps) => {
  const { t } = useT();
  return (
    <div className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3 flex flex-col gap-2">
      <div className="flex items-start gap-2">
        {server.icon_url ? (
          <img
            src={server.icon_url}
            alt=""
            className="w-8 h-8 rounded shrink-0 object-contain bg-white dark:bg-neutral-900"
          />
        ) : (
          <div className="w-8 h-8 rounded shrink-0 bg-primary-100 dark:bg-primary-500/20 flex items-center justify-center text-sm">
            🔌
          </div>
        )}
        <div className="flex-1 min-w-0">
          <div className="flex items-center gap-1.5 flex-wrap">
            <p className="text-sm font-medium text-stone-900 dark:text-neutral-100 truncate">
              {server.display_name}
            </p>
            {server.is_deployed && (
              <span className="shrink-0 px-1.5 py-0.5 text-[10px] border rounded-full bg-primary-50 dark:bg-primary-500/15 border-primary-200 dark:border-primary-500/30 text-primary-700 dark:text-primary-300">
                {t('mcp.catalog.deployed')}
              </span>
            )}
          </div>
          {server.use_count != null && server.use_count > 0 && (
            <p className="text-[11px] text-stone-400 dark:text-neutral-500 mt-0.5">
              {t('mcp.catalog.installCount').replace('{count}', server.use_count.toLocaleString())}
            </p>
          )}
        </div>
      </div>

      {server.description && (
        <p className="text-xs text-stone-500 dark:text-neutral-400 line-clamp-2">
          {server.description}
        </p>
      )}

      <button
        type="button"
        onClick={() => onInstall(server.qualified_name)}
        className="mt-auto w-full rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-primary-600 transition-colors">
        {t('mcp.install.button')}
      </button>
    </div>
  );
};

export default SmitheryServerCard;
