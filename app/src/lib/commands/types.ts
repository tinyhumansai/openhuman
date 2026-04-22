import type { ComponentType } from 'react';

export type ScopeKind = 'global' | 'page' | 'modal';
export type ShortcutString = string;

export interface ParsedShortcut {
  key: string;
  mod: boolean;
  shift: boolean;
  alt: boolean;
  ctrl: boolean;
}

export interface Action {
  id: string;
  label: string;
  hint?: string;
  group?: string;
  icon?: ComponentType<{ className?: string }>;
  shortcut?: ShortcutString;
  scope?: ScopeKind;
  enabled?: () => boolean;
  handler: () => void | Promise<void>;
  allowInInput?: boolean;
  repeat?: boolean;
  preventDefault?: boolean;
  keywords?: string[];
}

export interface RegisteredAction extends Action {
  scopeFrame: symbol;
}

export interface HotkeyBinding {
  shortcut: ShortcutString;
  handler: () => void;
  scope?: ScopeKind;
  enabled?: () => boolean;
  allowInInput?: boolean;
  repeat?: boolean;
  preventDefault?: boolean;
  description?: string;
  id?: string;
}

export interface ScopeFrame {
  symbol: symbol;
  id: string;
  kind: ScopeKind;
}

export interface ActiveBinding {
  frame: ScopeFrame;
  binding: HotkeyBinding;
  parsed: ParsedShortcut;
}
