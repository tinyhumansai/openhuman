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
import Button from '../../ui/Button';
import TextField from '../../ui/TextField';

interface McpServerSearchProps {
  value: string;
  onChange: (next: string) => void;
}

const McpServerSearch = ({ value, onChange }: McpServerSearchProps) => {
  const { t } = useT();
  const hasValue = value.length > 0;
  return (
    <div role="search" aria-label={t('mcp.installed.search.landmarkAria')} className="relative">
      <TextField
        type="search"
        inputSize="sm"
        value={value}
        onChange={e => onChange(e.target.value)}
        placeholder={t('mcp.installed.search.placeholder')}
        aria-label={t('mcp.installed.search.inputAria')}
        className="w-full pr-7 text-xs"
      />
      {hasValue && (
        <Button
          iconOnly
          variant="tertiary"
          size="xs"
          onClick={() => onChange('')}
          aria-label={t('mcp.installed.search.clearAria')}
          className="absolute right-1.5 top-1/2 -translate-y-1/2 h-auto w-auto rounded p-0.5 text-content-faint hover:bg-transparent hover:text-content-secondary">
          <svg
            className="w-3 h-3"
            fill="none"
            viewBox="0 0 24 24"
            stroke="currentColor"
            strokeWidth={2.5}
            aria-hidden="true">
            <path strokeLinecap="round" strokeLinejoin="round" d="M6 18L18 6M6 6l12 12" />
          </svg>
        </Button>
      )}
    </div>
  );
};

export default McpServerSearch;
