/**
 * SkillResourceTree
 * -----------------
 *
 * Groups a flat list of skill resource paths by their top-level directory
 * (`scripts/`, `references/`, `assets/`) with a catch-all "Other" bucket so
 * anything unexpected still renders. Items are rendered as clickable rows in
 * JetBrains Mono for path clarity. Selected item uses primary-50 background.
 */
import debug from 'debug';
import { useMemo } from 'react';

import { useT } from '../../lib/i18n/I18nContext';

const log = debug('skills:resource-tree');

interface Props {
  resources: string[];
  selectedPath: string | null;
  onSelect: (path: string) => void;
}

interface ResourceGroup {
  key: string;
  items: string[];
}

const KNOWN_GROUPS: Array<{ prefix: string; key: string }> = [
  { prefix: 'scripts/', key: 'scripts' },
  { prefix: 'references/', key: 'references' },
  { prefix: 'assets/', key: 'assets' },
  { prefix: 'templates/', key: 'templates' },
  { prefix: 'examples/', key: 'examples' },
  { prefix: 'prompts/', key: 'prompts' },
];

function groupResources(resources: string[]): ResourceGroup[] {
  const buckets = new Map<string, ResourceGroup>();
  for (const known of KNOWN_GROUPS) {
    buckets.set(known.key, { key: known.key, items: [] });
  }
  const other: ResourceGroup = { key: 'other', items: [] };

  for (const resource of resources) {
    let matched = false;
    for (const known of KNOWN_GROUPS) {
      if (resource.startsWith(known.prefix)) {
        buckets.get(known.key)!.items.push(resource);
        matched = true;
        break;
      }
    }
    if (!matched) {
      other.items.push(resource);
    }
  }

  for (const bucket of buckets.values()) {
    bucket.items.sort((a, b) => a.localeCompare(b));
  }
  other.items.sort((a, b) => a.localeCompare(b));

  const result: ResourceGroup[] = [];
  for (const known of KNOWN_GROUPS) {
    const bucket = buckets.get(known.key)!;
    if (bucket.items.length > 0) {
      result.push(bucket);
    }
  }
  if (other.items.length > 0) {
    result.push(other);
  }
  return result;
}

const GROUP_LABEL_KEYS: Record<string, string> = {
  scripts: 'skills.resource.tree.scripts',
  references: 'skills.resource.tree.references',
  assets: 'skills.resource.tree.assets',
  templates: 'skills.resource.tree.templates',
  examples: 'skills.resource.tree.examples',
  prompts: 'skills.resource.tree.prompts',
  other: 'skills.resource.tree.other',
};

export default function SkillResourceTree({ resources, selectedPath, onSelect }: Props) {
  const { t } = useT();
  const groups = useMemo(() => groupResources(resources), [resources]);

  if (groups.length === 0) {
    return (
      <p className="text-xs text-content-faint italic">
        {t('skills.resource.tree.empty')}
      </p>
    );
  }

  return (
    <div className="space-y-3">
      {groups.map(group => (
        <div
          key={group.key}
          className="rounded-xl border border-line bg-surface-muted overflow-hidden">
          <div className="flex items-center justify-between border-b border-line bg-surface-muted px-3 py-1.5">
            <h4 className="text-[11px] font-semibold uppercase tracking-wide text-content-secondary">
              {t(GROUP_LABEL_KEYS[group.key] ?? group.key)}
            </h4>
            <span className="text-[10px] text-content-faint font-mono">
              {group.items.length}
            </span>
          </div>
          <ul className="divide-y divide-line-subtle dark:divide-neutral-800">
            {group.items.map(path => {
              const isSelected = selectedPath === path;
              return (
                <li key={path}>
                  <button
                    type="button"
                    onClick={() => {
                      log('click path=%s', path);
                      onSelect(path);
                    }}
                    className={`w-full truncate px-3 py-2 text-left text-[11px] font-mono transition-colors focus:outline-none focus:ring-1 focus:ring-inset focus:ring-primary-500 ${
                      isSelected
                        ? 'bg-primary-50 dark:bg-primary-500/15 text-primary-700 dark:text-primary-300'
                        : 'text-content-secondary hover:bg-white dark:hover:bg-surface-muted/60'
                    }`}
                    title={path}>
                    {path}
                  </button>
                </li>
              );
            })}
          </ul>
        </div>
      ))}
    </div>
  );
}
