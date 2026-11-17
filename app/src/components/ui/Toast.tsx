/*
 * App-wide toasts, adapted from shadcn's Base UI toast (base-nova style,
 * https://ui.shadcn.com/docs/components/base/toast) onto this app's tokens,
 * `Button` and lucide icons.
 *
 * One `<Toaster />` is mounted at the app root. Anything can raise a toast
 * through the module-level manager without a hook or a prop:
 *
 *   toast.add({ type: 'success', title: 'Saved', description: '…' })
 *   toast.add({ title: '…', data: { icon: <Logo /> } })   // custom leading icon
 *   toast.promise(save(), { loading: '…', success: '…', error: '…' })
 *
 * Base UI owns stacking, swipe-to-dismiss, pause-on-hover and the live region.
 */
import { Toast as ToastPrimitive } from '@base-ui/react/toast';
import { CircleCheck, Info, Loader2, OctagonX, TriangleAlert, X } from 'lucide-react';
import type { ReactNode } from 'react';

import { cn } from '../../lib/cn';
import { useT } from '../../lib/i18n/I18nContext';
import Button from './Button';

/** Extra per-toast data: a leading icon that replaces the type's default. */
export interface ToastData {
  icon?: ReactNode;
}

const toastManager = ToastPrimitive.createToastManager<ToastData>();

/** A privacy-conscious record of recent toast events for diagnostics. */
export interface ToastLogEntry {
  id: string;
  type?: string;
  title: string;
  description?: string;
  timestamp: number;
}

const TOAST_LOG_LIMIT = 100;
const toastLog: ToastLogEntry[] = [];

/** Returns a copy of the most recent toast events, newest first. */
export function getToastLog(): ToastLogEntry[] {
  return toastLog.map(entry => ({ ...entry }));
}

/** Clears the in-memory diagnostic history. */
export function clearToastLog(): void {
  toastLog.length = 0;
}

/** The global toast manager also records every emitted toast for diagnostics. */
export const toast = Object.assign({}, toastManager, {
  add(options: Parameters<typeof toastManager.add>[0]) {
    const id = toastManager.add(options);
    const title = typeof options.title === 'string' ? options.title : '';
    const description = typeof options.description === 'string' ? options.description : undefined;
    toastLog.unshift({ id, type: options.type, title, description, timestamp: Date.now() });
    if (toastLog.length > TOAST_LOG_LIMIT) toastLog.length = TOAST_LOG_LIMIT;
    return id;
  },
});

type ToastType = 'success' | 'info' | 'warning' | 'error' | 'loading';

const TYPE_STYLES: Record<ToastType, { icon: ReactNode; tile: string; accent: string }> = {
  success: {
    icon: <CircleCheck />,
    tile: 'bg-sage-500/15 text-sage-600 dark:text-sage-300',
    accent: 'bg-sage-500',
  },
  info: {
    icon: <Info />,
    tile: 'bg-primary-500/15 text-primary-600 dark:text-primary-300',
    accent: 'bg-primary-500',
  },
  warning: {
    icon: <TriangleAlert />,
    tile: 'bg-amber-500/15 text-amber-600 dark:text-amber-300',
    accent: 'bg-amber-500',
  },
  error: {
    icon: <OctagonX />,
    tile: 'bg-coral-500/15 text-coral-600 dark:text-coral-300',
    accent: 'bg-coral-500',
  },
  loading: {
    icon: <Loader2 className="animate-spin" />,
    tile: 'bg-surface-strong text-content-secondary',
    accent: 'bg-line-strong',
  },
};

const typeStyle = (type: string | undefined) =>
  type && type in TYPE_STYLES ? TYPE_STYLES[type as ToastType] : null;

const ToastViewport = ({ className, ...props }: ToastPrimitive.Viewport.Props) => (
  <ToastPrimitive.Viewport
    data-slot="toast-viewport"
    className={cn(
      'pointer-events-none fixed inset-x-4 bottom-4 z-[60] mx-auto w-auto max-w-sm outline-none sm:right-4 sm:left-auto sm:mx-0 sm:w-full',
      className
    )}
    {...props}
  />
);

/** Base UI's stacking / swipe / enter-exit choreography, as shipped by shadcn. */
const ROOT_MOTION = [
  '[--gap:0.75rem] [--height:var(--toast-frontmost-height,var(--toast-height))] [--offset-y:calc(var(--toast-offset-y)*-1+calc(var(--toast-index)*var(--gap)*-1)+var(--toast-swipe-movement-y))] [--peek:0.75rem] [--scale:calc(max(0,1-(var(--toast-index)*0.1)))] [--shrink:calc(1-var(--scale))]',
  'h-(--height) [transform:translateX(var(--toast-swipe-movement-x))_translateY(calc(var(--toast-swipe-movement-y)-(var(--toast-index)*var(--peek))-(var(--shrink)*var(--height))))_scale(var(--scale))] [transition:transform_500ms_cubic-bezier(0.22,1,0.36,1),opacity_500ms,height_150ms]',
  "after:absolute after:top-full after:left-0 after:h-[calc(var(--gap)+1px)] after:w-full after:content-['']",
  'data-expanded:h-(--toast-height) data-expanded:[transform:translateX(var(--toast-swipe-movement-x))_translateY(var(--offset-y))]',
  'data-limited:opacity-0 data-starting-style:[transform:translateY(150%)]',
  '[&[data-ending-style]:not([data-limited]):not([data-swipe-direction])]:[transform:translateY(150%)]',
  'data-ending-style:data-[swipe-direction=down]:[transform:translateY(calc(var(--toast-swipe-movement-y)+150%))]',
  'data-ending-style:data-[swipe-direction=left]:[transform:translateX(calc(var(--toast-swipe-movement-x)-150%))_translateY(var(--offset-y))]',
  'data-ending-style:data-[swipe-direction=right]:[transform:translateX(calc(var(--toast-swipe-movement-x)+150%))_translateY(var(--offset-y))]',
  'data-ending-style:data-[swipe-direction=up]:[transform:translateY(calc(var(--toast-swipe-movement-y)-150%))]',
  'data-expanded:data-ending-style:data-[swipe-direction=down]:[transform:translateY(calc(var(--toast-swipe-movement-y)+150%))]',
  'data-expanded:data-ending-style:data-[swipe-direction=left]:[transform:translateX(calc(var(--toast-swipe-movement-x)-150%))_translateY(var(--offset-y))]',
  'data-expanded:data-ending-style:data-[swipe-direction=right]:[transform:translateX(calc(var(--toast-swipe-movement-x)+150%))_translateY(var(--offset-y))]',
  'data-expanded:data-ending-style:data-[swipe-direction=up]:[transform:translateY(calc(var(--toast-swipe-movement-y)-150%))]',
];

const ToastList = () => {
  const { t } = useT();
  const { toasts } = ToastPrimitive.useToastManager<ToastData>();

  return toasts.map(item => {
    const style = typeStyle(item.type);
    const icon = item.data?.icon ?? style?.icon;
    return (
      <ToastPrimitive.Root
        key={item.id}
        toast={item}
        data-slot="toast"
        data-testid="toast"
        data-type={item.type}
        className={cn(
          'group/toast pointer-events-auto absolute right-0 bottom-0 z-[calc(1000-var(--toast-index))] w-full origin-bottom overflow-hidden rounded-xl border border-line bg-surface text-content shadow-large will-change-transform outline-none select-none focus-visible:ring-2 focus-visible:ring-primary-500',
          ...ROOT_MOTION
        )}>
        {style && (
          <span aria-hidden className={cn('absolute inset-y-0 left-0 w-1', style.accent)} />
        )}
        <ToastPrimitive.Content
          data-slot="toast-content"
          className="flex h-full items-start gap-3 overflow-hidden py-3.5 pr-10 pl-4 transition-opacity duration-250 ease-[cubic-bezier(0.22,1,0.36,1)] data-behind:opacity-0 data-expanded:opacity-100">
          {icon && (
            <span
              aria-hidden
              data-slot="toast-icon"
              className={cn(
                'flex h-9 w-9 shrink-0 items-center justify-center rounded-lg [&_svg:not([class*=size-])]:h-4.5 [&_svg:not([class*=size-])]:w-4.5',
                style?.tile ?? 'bg-surface-strong text-content'
              )}>
              {icon}
            </span>
          )}
          <div className="flex min-w-0 flex-1 flex-col gap-0.5 self-center">
            <ToastPrimitive.Title
              data-slot="toast-title"
              className="text-sm leading-snug font-semibold text-content"
            />
            <ToastPrimitive.Description
              data-slot="toast-description"
              className="text-xs leading-relaxed break-words text-content-muted"
            />
            {item.actionProps && (
              <ToastPrimitive.Action
                data-slot="toast-action"
                render={<Button size="xs" variant="secondary" />}
                className="mt-2 self-start"
              />
            )}
          </div>
          <ToastPrimitive.Close
            data-slot="toast-close"
            aria-label={t('common.close')}
            render={<Button size="xs" variant="tertiary" iconOnly />}
            className="absolute top-2.5 right-2.5 text-content-muted hover:text-content">
            <X className="h-3.5 w-3.5" aria-hidden />
          </ToastPrimitive.Close>
        </ToastPrimitive.Content>
      </ToastPrimitive.Root>
    );
  });
};

/** Renders every toast raised through `toast`. Mount once, inside `I18nProvider`. */
export const Toaster = ({
  children,
  toastManager = toast,
  ...props
}: ToastPrimitive.Provider.Props) => (
  <ToastPrimitive.Provider toastManager={toastManager} {...props}>
    {children}
    <ToastPrimitive.Portal data-slot="toast-portal">
      <ToastViewport>
        <ToastList />
      </ToastViewport>
    </ToastPrimitive.Portal>
  </ToastPrimitive.Provider>
);

export default Toaster;
