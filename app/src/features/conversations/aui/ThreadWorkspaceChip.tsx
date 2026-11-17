/**
 * Store-connected {@link WorkspacePicker} for one thread. Renders only while
 * the thread has no messages — the core refuses to move a started thread —
 * and binds the choice through `updateThreadWorkingDir`.
 */
import debugFactory from 'debug';
import { useEffect, useMemo, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { useAppDispatch, useAppSelector } from '../../../store/hooks';
import { updateThreadWorkingDir } from '../../../store/threadSlice';
import { isTauri } from '../../../utils/tauriCommands/common';
import { openhumanGetAgentPaths } from '../../../utils/tauriCommands/config';
import { pickDirectoryNatively } from '../../../utils/tauriCommands/directoryPicker';
import { recentWorkingFolders, WorkspacePicker } from './WorkspacePicker';

const debug = debugFactory('conversations:workspace');

export function ThreadWorkspaceChip({ threadId }: { threadId: string | null }) {
  const { t } = useT();
  const dispatch = useAppDispatch();
  const threads = useAppSelector(state => state.thread.threads);
  const thread = threadId ? threads.find(candidate => candidate.id === threadId) : undefined;
  const [defaultDir, setDefaultDir] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  // Keyed by thread so a failure on one thread never shows on the next.
  const [failure, setFailure] = useState<{ threadId: string; message: string } | null>(null);

  useEffect(() => {
    let cancelled = false;
    void openhumanGetAgentPaths()
      .then(res => {
        if (!cancelled) setDefaultDir(res.result?.action_dir ?? null);
      })
      .catch((err: unknown) => {
        debug('agent paths unavailable: %s', err instanceof Error ? err.message : String(err));
      });
    return () => {
      cancelled = true;
    };
  }, []);

  const value = thread?.actionDir ?? null;
  const recent = useMemo(
    () => recentWorkingFolders(threads, [value, defaultDir]),
    [threads, value, defaultDir]
  );

  if (!thread || thread.messageCount > 0) return null;
  const error = failure?.threadId === thread.id ? failure.message : null;

  const bind = (dir: string | null) => {
    if (dir === value) return;
    setSaving(true);
    setFailure(null);
    debug('[chat][workspace] bind thread=%s default=%s', thread.id, dir === null);
    void dispatch(updateThreadWorkingDir({ threadId: thread.id, actionDir: dir }))
      .unwrap()
      .catch((err: unknown) => {
        debug('[chat][workspace] bind failed thread=%s', thread.id);
        setFailure({
          threadId: thread.id,
          message: typeof err === 'string' && err ? err : t('composer.workspace.error'),
        });
      })
      .finally(() => setSaving(false));
  };

  const chooseFolder = async () => {
    const picked = await pickDirectoryNatively();
    if (picked.ok) {
      bind(picked.path);
    } else if (picked.reason === 'failed') {
      setFailure({ threadId: thread.id, message: t('composer.workspace.error') });
    }
  };

  return (
    <WorkspacePicker
      value={value}
      defaultDir={defaultDir}
      recent={recent}
      onChange={bind}
      onChooseFolder={isTauri() ? () => void chooseFolder() : undefined}
      disabled={saving}
      error={error}
    />
  );
}

export default ThreadWorkspaceChip;
