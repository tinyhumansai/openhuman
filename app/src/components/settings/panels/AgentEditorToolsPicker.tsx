/**
 * AgentEditorToolsPicker — the "Allowed tools" field for AgentEditorPage.
 *
 * Renders the current selection as chips (or an "all tools" badge) plus an
 * "Add tools" trigger, and owns the searchable modal used to edit the
 * allowlist. Split out of AgentEditorPage.tsx to keep that file under the
 * repo's ~500-line guideline.
 */
import { useCallback, useEffect, useId, useMemo, useState } from 'react';
import { LuPlus, LuSearch } from 'react-icons/lu';

import { cn } from '../../../lib/cn';
import { useT } from '../../../lib/i18n/I18nContext';
import { agentRegistryApi, type AgentToolInfo } from '../../../services/api/agentRegistryApi';
import Badge from '../../ui/Badge';
import Button from '../../ui/Button';
import { CheckIcon, CloseIcon } from '../../ui/icons';
import { CenteredLoadingState } from '../../ui/LoadingState';
import { ModalShell } from '../../ui/ModalShell';
import TextField from '../../ui/TextField';
import Toggle from '../../ui/Toggle';

const ALL_TOOLS = '*';

/**
 * A decorative "is selected" indicator for a fully clickable row.
 *
 * Not `ui/Checkbox`: that renders a real `<input type="checkbox">`, and these
 * rows are themselves `Button`s — nesting an interactive form control inside
 * a button is invalid HTML. The row's own click/keyboard handling already
 * carries the toggle semantics, so this mark is purely visual and
 * `aria-hidden`.
 */
function ToggleMark({ checked, className }: { checked: boolean; className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        'flex h-4 w-4 flex-none items-center justify-center rounded border transition-colors',
        checked
          ? 'border-primary-500 bg-primary-500 text-content-inverted'
          : 'border-line-strong bg-surface',
        className
      )}>
      {checked && <CheckIcon className="h-3 w-3" />}
    </span>
  );
}

export interface AgentEditorToolsFieldProps {
  toolAllowlist: string[];
  onChange: (next: string[]) => void;
}

/** Chip row + "Add tools" trigger; opens `ToolsPickerModal` on demand. */
export function AgentEditorToolsField({ toolAllowlist, onChange }: AgentEditorToolsFieldProps) {
  const { t } = useT();
  const [toolsOpen, setToolsOpen] = useState(false);
  const allToolsSelected = toolAllowlist.length === 1 && toolAllowlist[0] === ALL_TOOLS;

  return (
    <div className="rounded-md border border-line p-2 dark:border-line-strong">
      <div className="flex flex-wrap items-center gap-1.5">
        {allToolsSelected ? (
          <Badge variant="primary">{t('settings.agents.editor.toolsAllSelected')}</Badge>
        ) : toolAllowlist.length === 0 ? (
          <span className="px-1 py-1 text-xs text-content-faint">
            {t('settings.agents.editor.toolsNoneSelected')}
          </span>
        ) : (
          toolAllowlist.map(tool => (
            <Badge key={tool} variant="neutral" className="gap-1 font-mono">
              {tool}
              <Button
                iconOnly
                variant="tertiary"
                size="xs"
                aria-label={t('settings.agents.editor.removeToolAria').replace('{tool}', tool)}
                onClick={() => onChange(toolAllowlist.filter(x => x !== tool))}
                className="h-3 w-3 min-w-0 rounded-full bg-transparent p-0 text-content-faint hover:bg-transparent hover:text-coral-600 dark:hover:text-coral-300">
                <CloseIcon className="h-3 w-3" />
              </Button>
            </Badge>
          ))
        )}
        <Button
          variant="tertiary"
          size="xs"
          aria-label={t('settings.agents.editor.selectTools')}
          onClick={() => setToolsOpen(true)}
          leadingIcon={<LuPlus className="h-3 w-3" />}
          className="h-auto gap-1 rounded-full border border-dashed border-line-strong bg-transparent px-2.5 py-1 text-xs font-medium text-content-secondary hover:border-primary-400 hover:bg-transparent hover:text-primary-600 dark:hover:border-primary-500 dark:hover:text-primary-300">
          {t('settings.agents.editor.selectTools')}
        </Button>
      </div>

      {toolsOpen && (
        <ToolsPickerModal
          allToolsSelected={allToolsSelected}
          selected={toolAllowlist}
          onToggleAll={() => onChange(toolAllowlist[0] === ALL_TOOLS ? [] : [ALL_TOOLS])}
          onToggleTool={tool => {
            const base = toolAllowlist[0] === ALL_TOOLS ? [] : toolAllowlist;
            onChange(base.includes(tool) ? base.filter(x => x !== tool) : [...base, tool]);
          }}
          onClose={() => setToolsOpen(false)}
        />
      )}
    </div>
  );
}

function ToolsPickerModal({
  allToolsSelected,
  selected,
  onToggleAll,
  onToggleTool,
  onClose,
}: {
  allToolsSelected: boolean;
  selected: string[];
  onToggleAll: () => void;
  onToggleTool: (tool: string) => void;
  onClose: () => void;
}) {
  const { t } = useT();
  const titleId = useId();
  const [tools, setTools] = useState<AgentToolInfo[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [query, setQuery] = useState('');

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      const list = await agentRegistryApi.availableTools();
      setTools(list);
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => {
    void load();
  }, [load]);

  const filtered = useMemo(() => {
    const q = query.trim().toLowerCase();
    if (!q) return tools;
    return tools.filter(
      tool => tool.name.toLowerCase().includes(q) || tool.description.toLowerCase().includes(q)
    );
  }, [tools, query]);

  const selectedCount = allToolsSelected ? tools.length : selected.length;

  return (
    <ModalShell
      title={t('settings.agents.editor.toolsModalTitle')}
      titleId={titleId}
      subtitle={t('settings.agents.editor.toolsSelectedCount').replace(
        '{count}',
        String(selectedCount)
      )}
      onClose={onClose}
      maxWidthClassName="max-w-lg"
      panelClassName="flex max-h-[calc(100vh-3rem)] flex-col"
      contentClassName="flex min-h-0 flex-1 flex-col p-0"
      footer={
        <div className="flex justify-end">
          <Button type="button" variant="primary" size="sm" onClick={onClose}>
            {t('settings.agents.editor.toolsDone')}
          </Button>
        </div>
      }>
      <>
        <div className="border-b border-line px-4 py-3">
          <div className="relative">
            <LuSearch className="pointer-events-none absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-content-faint" />
            <TextField
              autoFocus
              value={query}
              onChange={e => setQuery(e.target.value)}
              placeholder={t('settings.agents.editor.toolsSearchPlaceholder')}
              aria-label={t('settings.agents.editor.toolsSearchPlaceholder')}
              className="pl-8"
            />
          </div>

          <Toggle
            variant="secondary"
            pressed={allToolsSelected}
            onPressedChange={onToggleAll}
            data-testid="agent-tools-allow-all-toggle"
            className={
              'mt-2 h-auto w-full items-start justify-between gap-2 rounded-md px-3 py-2 text-left font-normal ' +
              (allToolsSelected
                ? 'border-primary-400 bg-primary-50 dark:border-primary-500/40 dark:bg-primary-500/10'
                : 'hover:bg-surface-hover')
            }>
            <span>
              <span className="block text-xs font-semibold text-content">
                {t('settings.agents.editor.toolsAllowAll')}
              </span>
              <span className="block text-[11px] text-content-faint">
                {t('settings.agents.editor.toolsAllowAllHint')}
              </span>
            </span>
            <ToggleMark checked={allToolsSelected} className="mt-0.5" />
          </Toggle>
        </div>

        <div className="min-h-32 flex-1 overflow-y-auto px-2 py-2">
          {loading ? (
            <CenteredLoadingState
              label={t('settings.agents.editor.toolsLoading')}
              className="py-10"
            />
          ) : error ? (
            <p className="px-2 py-6 text-center text-sm text-coral-600 dark:text-coral-300">
              {t('settings.agents.editor.toolsLoadError')}: {error}
            </p>
          ) : filtered.length === 0 ? (
            <p className="px-2 py-6 text-center text-sm text-content-faint">
              {t('settings.agents.editor.toolsEmpty')}
            </p>
          ) : (
            <ul>
              {filtered.map(tool => {
                const checked = allToolsSelected || selected.includes(tool.name);
                return (
                  <li key={tool.name}>
                    <Button
                      type="button"
                      variant="tertiary"
                      disabled={allToolsSelected}
                      aria-pressed={checked}
                      onClick={() => onToggleTool(tool.name)}
                      className="h-auto w-full items-start justify-start gap-3 rounded-md px-2 py-2 text-left font-normal disabled:cursor-not-allowed disabled:opacity-50">
                      <ToggleMark checked={checked} className="mt-0.5" />
                      <span className="min-w-0">
                        <span className="block font-mono text-xs font-medium text-content">
                          {tool.name}
                        </span>
                        <span className="block wrap-break-word text-[11px] leading-snug text-content-muted">
                          {tool.description}
                        </span>
                      </span>
                    </Button>
                  </li>
                );
              })}
            </ul>
          )}
        </div>
      </>
    </ModalShell>
  );
}
