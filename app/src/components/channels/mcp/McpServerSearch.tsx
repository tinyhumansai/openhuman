/**
 * Filter input for the installed MCP server list.
 *
 * Controlled component — the parent (`McpServersTab`) owns the value
 * and pushes it into `InstalledServerList` as the `filter` prop. The
 * input exposes a clear button when non-empty and announces itself
 * as a `role="search"` landmark so assistive tech can jump to it.
 *
 * Intentionally has NO global keyboard shortcut binding (e.g. Cmd/Ctrl+K)
 * to avoid clashing with the app-wide CommandProvider in `App.tsx`.
 * Users focus the input by clicking or tabbing.
 */
import { useT } from '../../../lib/i18n/I18nContext';

interface McpServerSearchProps {
  value: string;
  onChange: (next: string) => void;
}

const McpServerSearch = ({ value, onChange }: McpServerSearchProps) => {
  const { t } = useT();
  const hasValue = value.length > 0;
  return (
    <div role="search" aria-label={t('mcp.installed.search.landmarkAria')} className="relative">
      <input
        type="search"
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={t('mcp.installed.search.placeholder')}
        aria-label={t('mcp.installed.search.inputAria')}
        className="w-full rounded-lg border border-line bg-surface px-3 py-1.5 pr-7 text-xs text-content placeholder:text-stone-400 dark:placeholder:text-neutral-500 focus:outline-none focus:ring-2 focus:ring-primary-400 focus:border-primary-400"
      />
      {hasValue && (
        <button
          type="button"
          onClick={() => onChange('')}
          aria-label={t('mcp.installed.search.clearAria')}
          className="absolute right-1.5 top-1/2 -translate-y-1/2 rounded p-0.5 text-content-faint hover:text-content-secondary transition-colors">
          <svg
            className="w-3 h-3"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2.5}
            aria-hidden="true">
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </button>
      )}
    </div>
  );
};

export default McpServerSearch;
