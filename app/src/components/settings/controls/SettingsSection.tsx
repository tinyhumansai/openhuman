import { type ReactNode } from 'react';

export interface SettingsSectionProps {
  title?: string;
  description?: string;
  children: ReactNode;
  className?: string;
}

const SettingsSection = ({ title, description, children, className }: SettingsSectionProps) => {
  const base = 'rounded-xl border border-line bg-surface overflow-hidden';

  return (
    <div className={[base, className ?? ''].filter(Boolean).join(' ')}>
      {title && (
        <div className="px-4 pt-4 pb-0">
          {/* Real heading (h3, one level below SettingsHeader's h2) for a11y
              and so getByRole('heading') keeps resolving section titles. */}
          <h3 className="text-xs font-semibold tracking-wide text-content-muted">{title}</h3>
          {description && (
            <p className="mt-1 text-xs text-content-muted leading-relaxed">{description}</p>
          )}
        </div>
      )}
      <div className="divide-y divide-line-subtle dark:divide-neutral-800">{children}</div>
    </div>
  );
};

export default SettingsSection;
