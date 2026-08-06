import type { ReactNode } from 'react';

export interface FormActionsProps {
  children: ReactNode;
  align?: 'start' | 'end' | 'stretch';
  className?: string;
}

const ALIGNMENT_CLASSES: Record<NonNullable<FormActionsProps['align']>, string> = {
  start: 'justify-start',
  end: 'justify-end',
  stretch: 'items-stretch [&>*]:flex-1',
};

export default function FormActions({ children, align = 'end', className }: FormActionsProps) {
  return (
    <div className={['flex gap-2', ALIGNMENT_CLASSES[align], className].filter(Boolean).join(' ')}>
      {children}
    </div>
  );
}
