import { type ReactNode } from 'react';

export interface SettingsSectionProps {
  title?: string;
  description?: string;
  children: ReactNode;
  className?: string;
}

const SettingsSection = ({ title, description, children, className }: SettingsSectionProps) => {
  const base =
    'rounded-xl border border-neutral-200 dark:border-neutral-800 bg-white dark:bg-neutral-900 overflow-hidden';

  return (
    <div className={[base, className ?? ''].filter(Boolean).join(' ')}>
      {title && (
        <div className="px-4 pt-4 pb-0">
          {/* Real heading (h3, one level below SettingsHeader's h2) for a11y
              and so getByRole('heading') keeps resolving section titles. */}
          <h3 className="text-xs font-semibold tracking-wide text-neutral-500 dark:text-neutral-400">
            {title}
          </h3>
          {description && (
            <p className="mt-1 text-xs text-neutral-500 dark:text-neutral-400 leading-relaxed">
              {description}
            </p>
          )}
        </div>
      )}
      <div className="divide-y divide-neutral-100 dark:divide-neutral-800">{children}</div>
    </div>
  );
};

export default SettingsSection;
