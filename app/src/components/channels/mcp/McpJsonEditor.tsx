/**
 * `mcp.json` — the user's MCP servers as one editable document.
 *
 * The second tab of the MCP page, beside the server rows. Both write the
 * *same* store through the same core, so this is not an import/export format
 * that drifts from the real configuration: it is the configuration, in the
 * spelling a user already knows from `claude_desktop_config.json`. Pasting a
 * block of servers copied off a server's install page is one action here,
 * which is the whole reason a text surface earns its place. The registry's
 * one-step add for hosted servers writes into this same document.
 *
 * Three things this deliberately does *not* do:
 *
 * - **Guess at the core's rules.** Only JSON-ness and the document's shape are
 *   checked locally (see `mcpJson.ts`); a refusal from the core is shown
 *   verbatim, because the core is the authority on what it will dial.
 * - **Show a credential.** The read carries `envKeys` and `authConfigured`,
 *   never a value, so an entry with no `env` is not a server with no
 *   credential — it is a credential this surface cannot show. Saving such an
 *   entry leaves the stored value alone rather than clearing it, which is
 *   stated on screen because the opposite guess is the destructive one.
 * - **Reformat as you type.** The text is the user's until they press Revert.
 */
import debug from 'debug';
import { Loader2, RotateCcw, Save } from 'lucide-react';
import { useCallback, useEffect, useId, useMemo, useRef, useState } from 'react';

import { useT } from '../../../lib/i18n/I18nContext';
import { mcpClientsApi } from '../../../services/api/mcpClientsApi';
import { Alert, AlertDescription, AlertTitle } from '../../ui';
import Button from '../../ui/Button';
import TextArea from '../../ui/TextArea';
import { formatMcpConfig, mcpConfigChanged, parseMcpConfig } from './mcpJson';
import type { McpConfigDoc, McpConfigWriteResult } from './types';

const log = debug('mcp-clients:json');

interface McpJsonEditorProps {
  /** Called after a successful save, so the rows re-read what was written. */
  onSaved?: (result: McpConfigWriteResult) => void;
}

type Load = 'loading' | 'ready' | 'error';

const McpJsonEditor = ({ onSaved }: McpJsonEditorProps) => {
  const { t } = useT();
  const textareaId = useId();
  const [load, setLoad] = useState<Load>('loading');
  const [loaded, setLoaded] = useState<McpConfigDoc | null>(null);
  const [text, setText] = useState('');
  const [saving, setSaving] = useState(false);
  // The core's own refusal, kept until the next save rather than toasted: it
  // names an entry in a document still on screen, and a toast that names
  // `notion` is gone by the time the user finds `notion`.
  const [refusal, setRefusal] = useState<string | null>(null);
  const [lastWrite, setLastWrite] = useState<McpConfigWriteResult | null>(null);
  // Only the newest read may write its answer.
  const generation = useRef(0);

  // The initial state is already `loading`; a retry sets it again before
  // calling this, so the read itself never has to.
  const refresh = useCallback(async () => {
    const mine = ++generation.current;
    try {
      const doc = await mcpClientsApi.configGet();
      if (generation.current !== mine) return;
      log('loaded %d servers', Object.keys(doc.mcpServers).length);
      setLoaded(doc);
      setText(formatMcpConfig(doc));
      setLoad('ready');
    } catch (err) {
      if (generation.current !== mine) return;
      // We do not know what the user has, and an empty editor would invite a
      // save that wipes it.
      log('load failed: %s', err instanceof Error ? err.message : err);
      setLoad('error');
    }
  }, []);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  const parsed = useMemo(() => parseMcpConfig(text), [text]);
  const changed = useMemo(() => mcpConfigChanged(text, loaded), [text, loaded]);

  const parseMessage = useMemo(() => {
    if (parsed.ok) return null;
    return t(`mcp.json.parseError.${parsed.code}`)
      .replace('{name}', parsed.name ?? '')
      .replace('{detail}', parsed.detail ?? '');
  }, [parsed, t]);

  const handleSave = useCallback(async () => {
    if (!parsed.ok || saving) return;
    setSaving(true);
    setRefusal(null);
    try {
      const result = await mcpClientsApi.configSet(parsed.doc);
      const next: McpConfigDoc = { mcpServers: result.mcpServers };
      setLoaded(next);
      setText(formatMcpConfig(next));
      setLastWrite(result);
      onSaved?.(result);
    } catch (err) {
      setRefusal(err instanceof Error ? err.message : t('mcp.json.saveFailed'));
    } finally {
      setSaving(false);
    }
  }, [parsed, saving, onSaved, t]);

  const handleRevert = useCallback(() => {
    if (loaded) setText(formatMcpConfig(loaded));
    setRefusal(null);
  }, [loaded]);

  if (load === 'loading') {
    return (
      <div className="py-10 text-center text-sm text-content-faint" data-testid="mcp-json-loading">
        {t('mcp.json.loading')}
      </div>
    );
  }

  if (load === 'error') {
    return (
      <Alert variant="destructive" data-testid="mcp-json-error">
        <AlertTitle>{t('mcp.json.loadFailedTitle')}</AlertTitle>
        <AlertDescription>
          {t('mcp.json.loadFailedBody')}{' '}
          <Button
            variant="tertiary"
            size="xs"
            onClick={() => {
              setLoad('loading');
              void refresh();
            }}>
            {t('common.retry')}
          </Button>
        </AlertDescription>
      </Alert>
    );
  }

  return (
    <div className="space-y-3" data-testid="mcp-json-editor">
      <p className="text-sm text-content-muted">{t('mcp.json.intro')}</p>

      <TextArea
        id={textareaId}
        aria-label={t('mcp.json.editorAria')}
        data-testid="mcp-json-textarea"
        value={text}
        onChange={e => setText(e.target.value)}
        spellCheck={false}
        invalid={!parsed.ok && changed}
        className="min-h-72 font-mono text-xs leading-relaxed"
      />

      {/* Said beside the buffer rather than in the docs: `env` and `headers`
          are absent from every entry the core sends, and the reading a user
          would otherwise take from that — "this server has no credential" —
          is one keystroke away from pasting a token that was never missing. */}
      <p className="text-xs text-content-muted">{t('mcp.json.credentialsNote')}</p>

      {!parsed.ok && changed && parseMessage && (
        <Alert variant="destructive" density="compact" data-testid="mcp-json-parse-error">
          <AlertTitle>{t('mcp.json.invalidTitle')}</AlertTitle>
          <AlertDescription>{parseMessage}</AlertDescription>
        </Alert>
      )}

      {refusal && (
        <Alert variant="destructive" density="compact" data-testid="mcp-json-refusal">
          <AlertTitle>{t('mcp.json.refusedTitle')}</AlertTitle>
          <AlertDescription>{refusal}</AlertDescription>
        </Alert>
      )}

      <div className="flex flex-wrap items-center gap-2">
        <Button
          variant="primary"
          size="sm"
          disabled={!parsed.ok || !changed || saving}
          onClick={() => void handleSave()}
          leadingIcon={
            saving ? (
              <Loader2 className="size-4 animate-spin" aria-hidden="true" />
            ) : (
              <Save className="size-4" aria-hidden="true" />
            )
          }
          data-testid="mcp-json-save">
          {saving ? t('mcp.json.saving') : t('mcp.json.save')}
        </Button>
        <Button
          variant="tertiary"
          size="sm"
          disabled={!changed || saving}
          onClick={handleRevert}
          leadingIcon={<RotateCcw className="size-4" aria-hidden="true" />}
          data-testid="mcp-json-revert">
          {t('mcp.json.revert')}
        </Button>
        {changed && parsed.ok && (
          <span className="text-xs text-content-muted">{t('mcp.json.unsaved')}</span>
        )}
        {lastWrite && !changed && !refusal && (
          <span className="text-xs text-content-muted" data-testid="mcp-json-saved">
            {t('mcp.json.saved')
              .replace('{added}', String(lastWrite.added.length))
              .replace('{updated}', String(lastWrite.updated.length))
              .replace('{removed}', String(lastWrite.removed.length))}
          </span>
        )}
      </div>

      <details className="text-xs text-content-muted">
        <summary className="cursor-pointer select-none">{t('mcp.json.exampleTitle')}</summary>
        <pre className="mt-2 overflow-x-auto rounded-lg border border-line bg-surface-muted px-3 py-2 font-mono text-[11px] leading-relaxed text-content">
          {EXAMPLE}
        </pre>
      </details>
    </div>
  );
};

const EXAMPLE = `{
  "mcpServers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "~/Documents"]
    },
    "github": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-github"],
      "env": { "GITHUB_PERSONAL_ACCESS_TOKEN": "ghp_…" }
    },
    "hosted": {
      "url": "https://mcp.example.com/mcp",
      "headers": { "Authorization": "Bearer …" }
    }
  }
}`;

export default McpJsonEditor;
