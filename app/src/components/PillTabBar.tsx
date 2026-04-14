import type { ReactNode } from 'react';

interface PillTabBarItem<T extends string> {
  label: string;
  value: T;
}

interface PillTabBarProps<T extends string> {
  activeClassName?: string;
  containerClassName?: string;
  inactiveClassName?: string;
  items: PillTabBarItem<T>[];
  onChange: (value: T) => void;
  renderItem?: (item: PillTabBarItem<T>, active: boolean) => ReactNode;
  selected: T;
}

export default function PillTabBar<T extends string>({
  activeClassName = 'border-primary-200 bg-primary-50 text-primary-700',
  containerClassName = 'flex gap-2 overflow-x-auto pb-1 scrollbar-hide',
  inactiveClassName = 'border-stone-200 bg-white text-stone-600 hover:bg-stone-50',
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
            className={`flex-shrink-0 rounded-full border px-3 py-1 text-xs font-medium transition-colors ${
              active ? activeClassName : inactiveClassName
            }`}>
            {renderItem ? renderItem(item, active) : item.label}
          </button>
        );
      })}
    </div>
  );
}
