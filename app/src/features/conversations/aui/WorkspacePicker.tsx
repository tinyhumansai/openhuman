/**
 * The new-chat working-folder chip, shown above the composer while a thread
 * has no messages (after OpenClaw's "where this session works" target bar).
 *
 * It binds the folder the thread's agent acts in (`threads_update_working_dir`)
 * instead of the global `action_dir`. The core fixes the folder once the first
 * message is sent, because a resumed session keeps its first prompt, so the
 * chip disappears with the empty state rather than offering a change the core
 * would refuse.
 */
import {
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuRoot,
  DropdownMenuTrigger,
} from '../../../components/ui/DropdownMenu';
import { useT } from '../../../lib/i18n/I18nContext';
import { folderBasename } from '../threadList/groupThreads';

/** How many recently used folders the menu offers. */
export const RECENT_FOLDER_LIMIT = 5;

/**
 * Distinct folders other threads were started in, most recent first, without
 * the current choice or the default.
 */
export function recentWorkingFolders(
  threads: ReadonlyArray<{ actionDir?: string | null; lastMessageAt: string }>,
  exclude: ReadonlyArray<string | null | undefined>
): string[] {
  const skip = new Set(exclude.filter((dir): dir is string => Boolean(dir)));
  const seen = new Set<string>();
  const out: string[] = [];
  const sorted = [...threads].sort(
    (a, b) => Date.parse(b.lastMessageAt || '') - Date.parse(a.lastMessageAt || '')
  );
  for (const thread of sorted) {
    const dir = thread.actionDir?.trim();
    if (!dir || skip.has(dir) || seen.has(dir)) continue;
    seen.add(dir);
    out.push(dir);
    if (out.length >= RECENT_FOLDER_LIMIT) break;
  }
  return out;
}

function FolderIcon() {
  return (
    <svg
      className="h-3.5 w-3.5 flex-none"
      fill="none"
      stroke="currentColor"
      viewBox="0 0 24 24"
      aria-hidden="true">
      <path
        strokeLinecap="round"
        strokeLinejoin="round"
        strokeWidth={2}
        d="M3 7a2 2 0 012-2h4l2 2h8a2 2 0 012 2v8a2 2 0 01-2 2H5a2 2 0 01-2-2V7z"
      />
    </svg>
  );
}

export interface WorkspacePickerProps {
  /** The thread's bound folder; `null` means the global default. */
  value: string | null;
  /** The global `action_dir`, shown as the default choice when known. */
  defaultDir: string | null;
  /** Folders offered under "Recent" (see {@link recentWorkingFolders}). */
  recent: string[];
  /** Bind a folder, or `null` to go back to the default. */
  onChange: (dir: string | null) => void;
  /** Opens the native folder chooser; omitted where no host can (browser). */
  onChooseFolder?: () => void;
  disabled?: boolean;
  /** A failure from the last bind, shown beside the chip. */
  error?: string | null;
}

export function WorkspacePicker({
  value,
  defaultDir,
  recent,
  onChange,
  onChooseFolder,
  disabled = false,
  error = null,
}: WorkspacePickerProps) {
  const { t } = useT();
  const label = value
    ? folderBasename(value)
    : defaultDir
      ? t('composer.workspace.defaultWithName').replace('{folder}', folderBasename(defaultDir))
      : t('composer.workspace.default');
  return (
    <div className="flex min-w-0 items-center gap-2 px-1 pb-2" data-testid="composer-workspace">
      <DropdownMenuRoot>
        <DropdownMenuTrigger asChild disabled={disabled}>
          <button
            type="button"
            data-analytics-id="chat-workspace-picker"
            aria-label={t('composer.workspace.label')}
            title={value ?? defaultDir ?? t('composer.workspace.label')}
            className="inline-flex h-7 min-w-0 max-w-full cursor-pointer items-center gap-1.5 rounded-full border border-content-faint/35 px-3 text-xs font-medium text-content-muted transition-colors hover:border-primary-500/60 hover:bg-primary-500/10 hover:text-content-secondary disabled:cursor-default disabled:opacity-50">
            <FolderIcon />
            <span className="truncate" data-testid="composer-workspace-label">
              {label}
            </span>
            <svg
              className="h-3 w-3 flex-none"
              fill="none"
              stroke="currentColor"
              viewBox="0 0 24 24"
              aria-hidden="true">
              <path
                strokeLinecap="round"
                strokeLinejoin="round"
                strokeWidth={2}
                d="M19 9l-7 7-7-7"
              />
            </svg>
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="start" className="max-w-80">
          <DropdownMenuItem
            data-analytics-id="chat-workspace-default"
            data-testid="composer-workspace-default"
            onSelect={() => onChange(null)}
            title={defaultDir ?? undefined}>
            <FolderIcon />
            <span className="truncate">
              {defaultDir
                ? t('composer.workspace.defaultWithName').replace(
                    '{folder}',
                    folderBasename(defaultDir)
                  )
                : t('composer.workspace.default')}
            </span>
          </DropdownMenuItem>
          {recent.length > 0 && (
            <p className="px-2.5 pb-1 pt-2 text-[11px] font-medium uppercase tracking-wide text-content-faint">
              {t('composer.workspace.recent')}
            </p>
          )}
          {recent.map(dir => (
            <DropdownMenuItem
              key={dir}
              data-analytics-id="chat-workspace-recent"
              data-testid="composer-workspace-recent"
              onSelect={() => onChange(dir)}
              title={dir}>
              <FolderIcon />
              <span className="truncate">{folderBasename(dir)}</span>
            </DropdownMenuItem>
          ))}
          {onChooseFolder && (
            <DropdownMenuItem
              data-analytics-id="chat-workspace-choose"
              data-testid="composer-workspace-choose"
              onSelect={() => onChooseFolder()}>
              <span className="truncate">{t('composer.workspace.choose')}</span>
            </DropdownMenuItem>
          )}
        </DropdownMenuContent>
      </DropdownMenuRoot>
      {error ? (
        <span className="truncate text-xs text-coral-500" role="alert">
          {error}
        </span>
      ) : (
        <span className="truncate text-xs text-content-faint">{t('composer.workspace.hint')}</span>
      )}
    </div>
  );
}

export default WorkspacePicker;
