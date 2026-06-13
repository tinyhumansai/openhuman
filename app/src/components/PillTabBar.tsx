import type { ReactNode } from 'react';

interface PillTabBarItem<T extends string> {
  label: string;
  value: T;
}

interface PillTabBarProps<T extends string> {
  activeClassName?: string;
  containerClassName?: string;
  inactiveClassName?: string;
  itemClassName?: string;
  items: PillTabBarItem<T>[];
  onChange: (value: T) => void;
  renderItem?: (item: PillTabBarItem<T>, active: boolean) => ReactNode;
  selected: T;
}

export default function PillTabBar<T extends string>({
  activeClassName = 'border-primary-200 dark:border-primary-500/40 bg-primary-50 dark:bg-primary-500/15 text-primary-700 dark:text-primary-300',
  containerClassName = 'flex gap-2 overflow-x-auto pb-1 scrollbar-hide',
  inactiveClassName = 'border-stone-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 text-stone-600 dark:text-neutral-300 hover:bg-stone-50 dark:hover:bg-neutral-800/60',
  itemClassName = 'px-3',
  items,
  onChange,
  renderItem,
  selected,
}: PillTabBarProps<T>) {
  return (
    <div className={containerClassName} role="tablist">
      {items.map(item => {
        const active = selected === item.value;
        const tabId = `pill-tab-${String(item.value)}`;

        return (
          <button
            key={item.value}
            type="button"
            id={tabId}
            role="tab"
            aria-selected={active}
            tabIndex={active ? 0 : -1}
            onClick={() => onChange(item.value)}
            className={`flex-shrink-0 rounded-full border py-1 text-xs font-medium transition-colors ${itemClassName} ${
              active ? activeClassName : inactiveClassName
            }`}>
            {renderItem ? renderItem(item, active) : item.label}
          </button>
        );
      })}
    </div>
  );
}
