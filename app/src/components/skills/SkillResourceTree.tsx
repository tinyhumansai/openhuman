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

const log = debug('skills:resource-tree');

interface Props {
  resources: string[];
  selectedPath: string | null;
  onSelect: (path: string) => void;
}

interface ResourceGroup {
  label: string;
  key: string;
  items: string[];
}

const KNOWN_GROUPS: Array<{ prefix: string; label: string; key: string }> = [
  { prefix: 'scripts/', label: 'Scripts', key: 'scripts' },
  { prefix: 'references/', label: 'References', key: 'references' },
  { prefix: 'assets/', label: 'Assets', key: 'assets' },
];

function groupResources(resources: string[]): ResourceGroup[] {
  const buckets = new Map<string, ResourceGroup>();
  for (const known of KNOWN_GROUPS) {
    buckets.set(known.key, { label: known.label, key: known.key, items: [] });
  }
  const other: ResourceGroup = { label: 'Other', key: 'other', items: [] };

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

export default function SkillResourceTree({ resources, selectedPath, onSelect }: Props) {
  const groups = useMemo(() => groupResources(resources), [resources]);

  if (groups.length === 0) {
    return <p className="text-xs text-stone-400 italic">No bundled resources.</p>;
  }

  return (
    <div className="space-y-3">
      {groups.map(group => (
        <div
          key={group.key}
          className="rounded-xl border border-stone-200 bg-stone-50/50 overflow-hidden">
          <div className="flex items-center justify-between border-b border-stone-200 bg-stone-50 px-3 py-1.5">
            <h4 className="text-[11px] font-semibold uppercase tracking-wide text-stone-600">
              {group.label}
            </h4>
            <span className="text-[10px] text-stone-400 font-mono">{group.items.length}</span>
          </div>
          <ul className="divide-y divide-stone-100">
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
                        ? 'bg-primary-50 text-primary-700'
                        : 'text-stone-700 hover:bg-white'
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
