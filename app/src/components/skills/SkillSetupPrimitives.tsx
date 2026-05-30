import type { ReactNode } from 'react';

import { CheckIcon, ModalShell } from '../ui';

interface SkillSetupModalShellProps {
  children: ReactNode;
  onClose: () => void;
  title: ReactNode;
  titleId: string;
  subtitle?: ReactNode;
  icon: ReactNode;
}

export function SkillSetupModalShell({
  children,
  onClose,
  title,
  titleId,
  subtitle,
  icon,
}: SkillSetupModalShellProps) {
  return (
    <ModalShell onClose={onClose} title={title} titleId={titleId} subtitle={subtitle} icon={icon}>
      {children}
    </ModalShell>
  );
}

interface SetupNoticeProps {
  children: ReactNode;
  tone?: 'sage' | 'amber' | 'coral';
  icon?: ReactNode;
  className?: string;
}

const NOTICE_CLASSES = {
  sage: 'border-sage-200 bg-sage-50 text-sage-700',
  amber: 'border-amber-200 bg-amber-50 text-amber-700',
  coral: 'border-coral-200 bg-coral-50 text-coral-700',
};

export function SetupNotice({ children, tone = 'sage', icon, className }: SetupNoticeProps) {
  return (
    <div
      className={`rounded-xl border p-3 text-xs ${NOTICE_CLASSES[tone]} ${
        icon ? 'flex items-start gap-2' : ''
      } ${className ?? ''}`}>
      {icon ? <span className="mt-0.5 flex-shrink-0">{icon}</span> : null}
      <div className="min-w-0">{children}</div>
    </div>
  );
}

interface SetupSettingRowProps {
  label: ReactNode;
  value: ReactNode;
  mono?: boolean;
}

export function SetupSettingRow({ label, value, mono = false }: SetupSettingRowProps) {
  return (
    <div className="flex items-center justify-between rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-3 py-2.5">
      <span className="text-sm text-stone-700 dark:text-neutral-200">{label}</span>
      <span className={`text-xs text-stone-500 dark:text-neutral-400 ${mono ? 'font-mono' : ''}`}>
        {value}
      </span>
    </div>
  );
}

interface SetupSuccessProps {
  title: ReactNode;
  description: ReactNode;
  settingsLabel: ReactNode;
  finishLabel: ReactNode;
  onSettings: () => void;
  onFinish: () => void;
}

export function SetupSuccess({
  title,
  description,
  settingsLabel,
  finishLabel,
  onSettings,
  onFinish,
}: SetupSuccessProps) {
  return (
    <div className="space-y-4 text-center py-2">
      <div className="mx-auto w-12 h-12 rounded-full bg-sage-50 flex items-center justify-center">
        <CheckIcon className="w-6 h-6 text-sage-500" />
      </div>

      <div>
        <h3 className="text-sm font-semibold text-stone-900 dark:text-neutral-100">{title}</h3>
        <p className="mt-1 text-xs text-stone-500 dark:text-neutral-400 leading-relaxed">
          {description}
        </p>
      </div>

      <div className="flex flex-col gap-2">
        <button
          type="button"
          onClick={onSettings}
          className="w-full rounded-xl border border-primary-200 bg-primary-50 px-4 py-2.5 text-sm font-medium text-primary-700 hover:bg-primary-100 transition-colors">
          {settingsLabel}
        </button>
        <button
          type="button"
          onClick={onFinish}
          className="w-full rounded-xl border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 px-4 py-2.5 text-sm font-medium text-stone-600 dark:text-neutral-300 hover:bg-stone-100 dark:hover:bg-neutral-800 dark:bg-neutral-800 transition-colors">
          {finishLabel}
        </button>
      </div>
    </div>
  );
}
