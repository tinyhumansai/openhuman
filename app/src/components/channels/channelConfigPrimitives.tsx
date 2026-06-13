import { type ReactNode, useCallback, useState } from 'react';

import type { AuthModeSpec, ChannelConnectionStatus } from '../../types/channels';
import ChannelFieldInput from './ChannelFieldInput';
import ChannelStatusBadge from './ChannelStatusBadge';

export function useChannelAuthFormState() {
  const [busyKeys, setBusyKeys] = useState<Record<string, boolean>>({});
  const [fieldValues, setFieldValues] = useState<Record<string, Record<string, string>>>({});
  const [error, setError] = useState<string | null>(null);

  const runBusy = useCallback(async (key: string, task: () => Promise<void>) => {
    setBusyKeys(prev => ({ ...prev, [key]: true }));
    setError(null);
    try {
      await task();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusyKeys(prev => ({ ...prev, [key]: false }));
    }
  }, []);

  const updateField = useCallback((compositeKey: string, fieldKey: string, value: string) => {
    setFieldValues(prev => ({
      ...prev,
      [compositeKey]: { ...(prev[compositeKey] ?? {}), [fieldKey]: value },
    }));
  }, []);

  return { busyKeys, fieldValues, error, setError, runBusy, updateField };
}

interface ChannelConfigErrorProps {
  message: string;
}

export function ChannelConfigError({ message }: ChannelConfigErrorProps) {
  return (
    <div className="rounded-lg border border-coral-200 dark:border-coral-500/30 bg-coral-50 dark:bg-coral-500/10 px-4 py-3 text-sm text-coral-700 dark:text-coral-300">
      {message}
    </div>
  );
}

interface ChannelAuthModeCardProps {
  children: ReactNode;
  title?: ReactNode;
  description: ReactNode;
  status: ChannelConnectionStatus;
  lastError?: string;
}

export function ChannelAuthModeCard({
  children,
  title,
  description,
  status,
  lastError,
}: ChannelAuthModeCardProps) {
  return (
    <div className="rounded-lg border border-stone-200 dark:border-neutral-800 bg-stone-50 dark:bg-neutral-800/60 p-3">
      <div className="flex items-start justify-between gap-3">
        <div>
          {title ? (
            <p className="text-sm font-medium text-stone-900 dark:text-neutral-100">{title}</p>
          ) : null}
          <p className={`text-xs text-stone-500 dark:text-neutral-400 ${title ? 'mt-1' : ''}`}>
            {description}
          </p>
          {lastError ? <p className="text-xs text-coral-600 mt-1">{lastError}</p> : null}
        </div>
        <ChannelStatusBadge status={status} />
      </div>
      {children}
    </div>
  );
}

interface ChannelAuthFieldsProps {
  spec: AuthModeSpec;
  compositeKey: string;
  fieldValues: Record<string, Record<string, string>>;
  onChange: (compositeKey: string, fieldKey: string, value: string) => void;
  disabled?: boolean;
  mapField?: (field: AuthModeSpec['fields'][number]) => AuthModeSpec['fields'][number];
}

export function ChannelAuthFields({
  spec,
  compositeKey,
  fieldValues,
  onChange,
  disabled,
  mapField,
}: ChannelAuthFieldsProps) {
  if (spec.fields.length === 0) return null;

  return (
    <div className="mt-3 space-y-2">
      {spec.fields.map(field => {
        const mapped = mapField ? mapField(field) : field;
        return (
          <ChannelFieldInput
            key={field.key}
            field={mapped}
            value={fieldValues[compositeKey]?.[field.key] ?? ''}
            onChange={val => onChange(compositeKey, field.key, val)}
            disabled={disabled}
          />
        );
      })}
    </div>
  );
}

interface ChannelConnectActionsProps {
  busy?: boolean;
  status: ChannelConnectionStatus;
  connectLabel: ReactNode;
  disconnectLabel: ReactNode;
  onConnect?: () => void;
  onDisconnect: () => void;
  showConnect?: boolean;
  className?: string;
}

export function ChannelConnectActions({
  busy,
  status,
  connectLabel,
  disconnectLabel,
  onConnect,
  onDisconnect,
  showConnect = status !== 'connected',
  className,
}: ChannelConnectActionsProps) {
  return (
    <div className={`mt-3 flex gap-2 ${className ?? ''}`}>
      {showConnect && onConnect ? (
        <button
          type="button"
          disabled={busy}
          onClick={onConnect}
          className="rounded-lg bg-primary-500 px-3 py-1.5 text-xs font-medium text-white hover:bg-primary-600 disabled:opacity-50">
          {connectLabel}
        </button>
      ) : null}
      <button
        type="button"
        disabled={busy || status === 'disconnected'}
        onClick={onDisconnect}
        className="rounded-lg border border-stone-200 dark:border-neutral-800 px-3 py-1.5 text-xs font-medium text-stone-600 dark:text-neutral-300 hover:border-stone-300 dark:hover:border-neutral-700 disabled:opacity-50">
        {disconnectLabel}
      </button>
    </div>
  );
}
