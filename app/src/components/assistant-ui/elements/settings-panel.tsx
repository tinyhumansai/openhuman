'use client';

/**
 * A model switcher, a system prompt, a temperature slider and a column of
 * on/off switches in one card.
 *
 * Vendored from the assistant-ui `elements-settings-panel` registry item
 * (https://r.assistant-ui.com/styles/base-nova/elements-settings-panel.json).
 * Changes from upstream:
 * - `cn` import path (`@/components/assistant-ui/lib/utils`).
 * - The "model", "system prompt" and "temperature" captions and the textarea /
 *   slider accessible names are props with English defaults, for `useT()` —
 *   see `ChatSettingsPanel` in `features/conversations/aui/ChatSettingsPanel.tsx`.
 * - `systemPrompt` and `temperature` are optional, and the toggle column
 *   renders only when it has entries: a section with no value is omitted, so
 *   a host shows only the settings it can actually persist rather than a
 *   control that writes nowhere.
 */
import { cn } from '@/components/assistant-ui/lib/utils';
import type { ComponentProps } from 'react';

import { clamp } from '../utils/range';
import { field, mono, paper } from './surfaces';

export interface SettingToggle {
  key: string;
  label: string;
  detail: string;
  on: boolean;
}

export function SettingsPanel({
  model,
  models,
  systemPrompt,
  temperature,
  toggles = [],
  onModelChange,
  onSystemPromptChange,
  onTemperatureChange,
  onToggle,
  modelLabel = 'model',
  systemPromptLabel = 'system prompt',
  systemPromptAriaLabel = 'System prompt',
  temperatureLabel = 'temperature',
  temperatureAriaLabel = 'Temperature',
  className,
  ...props
}: Omit<
  ComponentProps<'div'>,
  | 'children'
  | 'model'
  | 'models'
  | 'systemPrompt'
  | 'temperature'
  | 'toggles'
  | 'onModelChange'
  | 'onSystemPromptChange'
  | 'onTemperatureChange'
  | 'onToggle'
> & {
  model: string;
  models: readonly string[];
  systemPrompt?: string;
  temperature?: number;
  toggles?: readonly SettingToggle[];
  onModelChange?: (model: string) => void;
  onSystemPromptChange?: (prompt: string) => void;
  onTemperatureChange?: (temperature: number) => void;
  onToggle?: (key: string) => void;
  modelLabel?: string;
  systemPromptLabel?: string;
  systemPromptAriaLabel?: string;
  temperatureLabel?: string;
  temperatureAriaLabel?: string;
}) {
  return (
    <div
      data-slot="settings-panel"
      className={cn(paper, 'flex w-full flex-col gap-4 rounded-[20px] p-4', className)}
      {...props}>
      <div className="flex flex-col gap-1.5">
        <span className={cn(mono, 'text-muted-foreground')}>{modelLabel}</span>
        <div className={cn(field, 'flex gap-0.5 rounded-full p-0.5')}>
          {models.map(option => {
            const className = cn(
              'flex-1 rounded-full py-1 text-xs font-medium transition-[background-color,color,scale] duration-150',
              onModelChange && 'active:scale-[0.97]',
              option === model
                ? 'bg-background text-foreground/90'
                : onModelChange
                  ? 'text-muted-foreground hover:text-muted-foreground'
                  : 'text-muted-foreground'
            );

            return onModelChange ? (
              <button
                key={option}
                type="button"
                aria-pressed={option === model}
                onClick={() => onModelChange(option)}
                className={className}>
                {option}
              </button>
            ) : (
              <span
                key={option}
                aria-current={option === model ? 'true' : undefined}
                className={className}>
                {option}
              </span>
            );
          })}
        </div>
      </div>

      {systemPrompt !== undefined && (
        <div className="flex flex-col gap-1.5">
          <span className={cn(mono, 'text-muted-foreground')}>{systemPromptLabel}</span>
          <textarea
            value={systemPrompt}
            onChange={event => onSystemPromptChange?.(event.target.value)}
            rows={3}
            aria-label={systemPromptAriaLabel}
            className={cn(
              field,
              'text-foreground/80 focus-visible:ring-foreground/20 resize-none rounded-xl px-3 py-2 text-xs leading-relaxed outline-none focus-visible:ring-1'
            )}
          />
        </div>
      )}

      {temperature !== undefined && (
        <div className="flex flex-col gap-1.5">
          <span className="flex items-baseline justify-between">
            <span className={cn(mono, 'text-muted-foreground')}>{temperatureLabel}</span>
            <span className={cn(mono, 'text-muted-foreground tabular-nums')}>
              {clamp(temperature, 0, 2).toFixed(1)}
            </span>
          </span>
          <input
            type="range"
            min={0}
            max={2}
            step={0.1}
            value={clamp(temperature, 0, 2)}
            aria-label={temperatureAriaLabel}
            onChange={event => onTemperatureChange?.(Number(event.target.value))}
            className="accent-foreground/80 h-1 w-full cursor-pointer"
          />
        </div>
      )}

      {toggles.length > 0 && (
        <div className="flex flex-col gap-2.5">
          {toggles.map(toggle => (
            <div key={toggle.key} className="flex items-center gap-3">
              <span className="flex min-w-0 flex-1 flex-col">
                <span className="truncate text-[13px]">{toggle.label}</span>
                <span className="text-muted-foreground truncate text-xs">{toggle.detail}</span>
              </span>
              {onToggle ? (
                <button
                  type="button"
                  role="switch"
                  aria-checked={toggle.on}
                  aria-label={toggle.label}
                  onClick={() => onToggle(toggle.key)}
                  className={cn(
                    'flex h-5 w-9 shrink-0 items-center rounded-full p-0.5 transition-colors duration-200',
                    toggle.on ? 'bg-foreground/80' : 'bg-foreground/15'
                  )}>
                  <span
                    className={cn(
                      'bg-background size-4 rounded-full transition-transform duration-200 motion-reduce:transition-none',
                      toggle.on && 'translate-x-4'
                    )}
                  />
                </button>
              ) : (
                <span
                  role="switch"
                  aria-checked={toggle.on}
                  aria-disabled="true"
                  aria-label={toggle.label}
                  className={cn(
                    'flex h-5 w-9 shrink-0 items-center rounded-full p-0.5',
                    toggle.on ? 'bg-foreground/80' : 'bg-foreground/15'
                  )}>
                  <span
                    className={cn(
                      'bg-background size-4 rounded-full',
                      toggle.on && 'translate-x-4'
                    )}
                  />
                </span>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
