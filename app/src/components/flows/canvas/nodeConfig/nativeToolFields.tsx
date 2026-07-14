/**
 * NativeToolField — the tool picker for the "Tool" node (native OpenHuman tools,
 * as opposed to the Composio "App action" node). Loads the agent's tool
 * registry from `listRuntimeTools` (`openhuman.javascript_list_tools`) and lets
 * the author pick one from a dropdown; the value stored on `config.slug` is
 * `oh:<tool_name>`, which the flow engine routes to the native tool registry.
 *
 * Shows the selected tool's description + a peek at its parameters so the author
 * knows what `args` to supply. Fetches are guarded — outside Tauri / on error it
 * falls back to a raw text input so a slug is never a dead end.
 */
import { useEffect, useMemo, useState } from 'react';

import { useT } from '../../../../lib/i18n/I18nContext';
import { listRuntimeTools, type RuntimeTool } from '../../../../services/api/runtimeToolsApi';
import { Field, INPUT_CLASS, MONO_CLASS } from './nodeConfigFields';

/** The `oh:` prefix that marks a native-tool slug (mirrors the Rust constant). */
export const NATIVE_TOOL_PREFIX = 'oh:';

export interface NativeToolFieldProps {
  label: string;
  hint?: string;
  /** The full `config.slug`, e.g. `oh:web_search` (or empty / `oh:`). */
  value: string;
  onChange: (value: string) => void;
  testId?: string;
}

/** Strip the `oh:` prefix to the bare tool name. */
function toolName(slug: string): string {
  return slug.startsWith(NATIVE_TOOL_PREFIX) ? slug.slice(NATIVE_TOOL_PREFIX.length) : slug;
}

export function NativeToolField({ label, hint, value, onChange, testId }: NativeToolFieldProps) {
  const { t } = useT();
  const [tools, setTools] = useState<RuntimeTool[]>([]);
  const [loading, setLoading] = useState(true);
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    void (async () => {
      try {
        const list = await listRuntimeTools();
        if (!cancelled) setTools(list);
      } catch {
        if (!cancelled) setFailed(true);
      } finally {
        if (!cancelled) setLoading(false);
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const current = toolName(value);
  const selected = useMemo(() => tools.find(tool => tool.name === current), [tools, current]);

  // Catalog unavailable → raw text input so the author can still enter a slug.
  if (failed && tools.length === 0) {
    return (
      <Field label={label} hint={hint}>
        <input
          type="text"
          className={`${INPUT_CLASS} ${MONO_CLASS}`}
          value={current}
          placeholder="web_search"
          data-testid={testId ? `${testId}-custom` : undefined}
          onChange={e =>
            onChange(e.target.value ? `${NATIVE_TOOL_PREFIX}${e.target.value.trim()}` : '')
          }
        />
      </Field>
    );
  }

  // Keep a saved-but-unknown tool selectable.
  const names = current && !tools.some(tk => tk.name === current) ? [current] : [];
  const options = [...names, ...tools.map(tk => tk.name)];

  return (
    <Field label={label} hint={hint}>
      <div className="space-y-1.5">
        <select
          className={INPUT_CLASS}
          value={current}
          disabled={loading}
          data-testid={testId}
          onChange={e => onChange(e.target.value ? `${NATIVE_TOOL_PREFIX}${e.target.value}` : '')}>
          <option value="">
            {loading ? t('flows.nodeConfig.native.loading') : t('flows.nodeConfig.native.select')}
          </option>
          {options.map(name => (
            <option key={name} value={name}>
              {name}
            </option>
          ))}
        </select>
        {selected?.description && (
          <p
            className="text-[11px] leading-snug text-content-muted"
            data-testid="node-config-native-desc">
            {selected.description}
          </p>
        )}
      </div>
    </Field>
  );
}
