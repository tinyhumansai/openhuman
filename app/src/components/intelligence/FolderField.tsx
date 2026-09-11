/**
 * Path field for the `folder` memory-source kind, with a Browse button backed
 * by the OS-native directory chooser.
 *
 * Split out of `AddMemorySourceFields` for the same reason that file was split
 * out of the dialog: it is the only field with behaviour of its own, and the
 * reasoning below is most of what there is to say about it.
 *
 * Browse used to be an `<input type="file" webkitdirectory>`. That element
 * cannot report where the directory it returned actually lives - `File.path`
 * is an Electron extension no web engine implements, Wry's WKWebView,
 * WebView2 and WebKitGTK included - so its handler fell through to
 * `webkitRelativePath.split('/')[0]` and stored the directory's bare *name*.
 * The source then looked configured and failed every sync cycle, forever,
 * with `folder does not exist: docs` (#5831).
 *
 * The rule that replaces it: **never store a value that cannot resolve.**
 * When no absolute path can be obtained the field says so and stays as it
 * was, because a visible error is recoverable and a silently stored name is
 * not. Typing a path by hand is untouched, including a relative one - the
 * reader anchors those on the workspace at read time (tinymemory#113), which
 * is deliberately not something this field second-guesses.
 */
import debug from 'debug';
import { useState } from 'react';

import { useT } from '../../lib/i18n/I18nContext';
import { pickDirectoryNatively } from '../../utils/tauriCommands/directoryPicker';
import Button from '../ui/Button';
import TextField from '../ui/TextField';

const log = debug('intelligence:folder-field');

export interface FolderFieldProps {
  label: string;
  value: string;
  onChange: (v: string) => void;
}

export function FolderField({ label, value, onChange }: FolderFieldProps) {
  const { t } = useT();
  const [pickError, setPickError] = useState<string | null>(null);

  const handleBrowse = async () => {
    const result = await pickDirectoryNatively();
    if (result.ok) {
      setPickError(null);
      onChange(result.path);
      return;
    }
    if (result.reason === 'cancelled') {
      // Dismissing the dialog is not a failure. Leave the field untouched and
      // clear any error from an earlier attempt.
      setPickError(null);
      return;
    }
    log('folder picker produced no absolute path (%s); refusing to store a name', result.reason);
    setPickError(t('memorySources.folderPathUnavailable'));
  };

  return (
    <label className="block">
      <span className="text-xs font-medium text-content-secondary">{label}</span>
      <div className="mt-1 flex gap-2">
        <TextField
          type="text"
          value={value}
          onChange={e => {
            setPickError(null);
            onChange(e.target.value);
          }}
          placeholder={t('memorySources.folderPathPlaceholder')}
        />
        <Button
          type="button"
          variant="secondary"
          size="sm"
          className="shrink-0"
          analyticsId="brain-sources-folder-browse"
          onClick={handleBrowse}>
          {t('memorySources.browse')}
        </Button>
      </div>
      {pickError && (
        <p role="alert" className="mt-1 text-xs leading-5 text-red-600 dark:text-red-300">
          {pickError}
        </p>
      )}
    </label>
  );
}

export default FolderField;
