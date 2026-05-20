/**
 * SkillResourceTree
 * -----------------
 *
 * Groups a flat list of skill resource paths by their top-level directory
 * (`scripts/`, `references/`, `assets/`) with a catch-all "Other" bucket so
 * anything unexpected still renders. Items are rendered as clickable rows in
 * JetBrains Mono for path clarity. Selected item uses primary-50 background.
 */
import { useMemo } from 'react';
import debug from 'debug';

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
  other: 'skills.resource.tree.other',
};

export default function SkillResourceTree({ resources, selectedPath, onSelect }: Props) {
  const { t } = useT();
  const groups = useMemo(() => groupResources(resources), [resources]);

  if (groups.length === 0) {
    return <p className="text-xs text-stone-400 dark:text-neutral-500 italic">{t('skills.resource.tree.empty')}</p>;
  }

  return (
    <div className="space-y-3">
      {groups.map(group => (
        <div
          key={group.key}
          className="rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 overflow-hidden">
          <div className="flex items-center justify-between border-b border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-1.5">
            <h4 className="text-[11px] font-semibold uppercase tracking-wide text-stone-600 dark:text-neutral-300">
              {t(GROUP_LABEL_KEYS[group.key] ?? group.key)}
            </h4>
            <span className="text-[10px] text-stone-400 dark:text-neutral-500 font-mono">{group.items.length}</span>
          </div>
          <ul className="divide-y divide-stone-100 dark:divide-neutral-800">
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
                        : 'text-stone-700 dark:text-neutral-200 hover:bg-white dark:hover:bg-neutral-800/60'
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
