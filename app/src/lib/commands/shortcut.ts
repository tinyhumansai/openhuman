import type { ParsedShortcut, ShortcutString } from './types';

const MODIFIER_TOKENS = new Set(['mod', 'shift', 'alt', 'ctrl']);
const NAMED_KEYS = new Set([
  'escape',
  'enter',
  'tab',
  'space',
  'arrowup',
  'arrowdown',
  'arrowleft',
  'arrowright',
  'backspace',
  'delete',
  'home',
  'end',
  'pageup',
  'pagedown',
  'f1',
  'f2',
  'f3',
  'f4',
  'f5',
  'f6',
  'f7',
  'f8',
  'f9',
  'f10',
  'f11',
  'f12',
]);

const parseCache = new Map<string, ParsedShortcut>();

export function parseShortcut(raw: ShortcutString): ParsedShortcut {
  const cached = parseCache.get(raw);
  if (cached) return cached;
  if (!raw) throw new Error('parseShortcut: empty shortcut string');
  const tokens = raw
    .toLowerCase()
    .split('+')
    .map((t) => t.trim())
    .filter(Boolean);
  if (tokens.length === 0) throw new Error(`parseShortcut: invalid shortcut "${raw}"`);
  const key = tokens[tokens.length - 1];
  if (MODIFIER_TOKENS.has(key)) throw new Error(`parseShortcut: shortcut "${raw}" has no key`);
  if (key === 'meta' || key === 'cmd' || key === 'command') {
    throw new Error(`parseShortcut: use "mod" instead of "${key}" in "${raw}"`);
  }
  if (
    key.length > 1 &&
    !NAMED_KEYS.has(key) &&
    !/^[a-z0-9]$/.test(key) &&
    !/^[\p{P}\p{S}]$/u.test(key)
  ) {
    throw new Error(`parseShortcut: unknown key "${key}" in "${raw}"`);
  }
  const result: ParsedShortcut = { key, mod: false, shift: false, alt: false, ctrl: false };
  for (let i = 0; i < tokens.length - 1; i++) {
    const m = tokens[i];
    if (m === 'mod' || m === 'shift' || m === 'alt' || m === 'ctrl') {
      result[m] = true;
    } else {
      throw new Error(`parseShortcut: unknown modifier "${m}" in "${raw}"`);
    }
  }
  parseCache.set(raw, result);
  return result;
}

export function isMac(): boolean {
  if (typeof navigator === 'undefined') return false;
  return navigator.platform.toLowerCase().includes('mac');
}

export function matchEvent(parsed: ParsedShortcut, e: KeyboardEvent): boolean {
  const mac = isMac();
  const modPressed = mac ? e.metaKey : e.ctrlKey;
  if (parsed.mod !== modPressed) return false;
  // Explicit ctrl flag tracks e.ctrlKey on mac (where ctrl is independent of mod).
  // On non-mac, ctrl IS the mod, so explicit ctrl must equal mod.
  if (mac) {
    if (parsed.ctrl !== e.ctrlKey) return false;
  } else {
    // non-mac: don't double-check ctrl when mod is set (mod === ctrl)
    if (!parsed.mod && parsed.ctrl !== e.ctrlKey) return false;
  }
  if (parsed.shift !== e.shiftKey) return false;
  if (parsed.alt !== e.altKey) return false;
  const eventKey = e.key.toLowerCase();
  return eventKey === parsed.key;
}

const MAC_GLYPHS: Record<string, string> = {
  mod: '⌘',
  shift: '⇧',
  alt: '⌥',
  ctrl: '⌃',
  escape: 'Esc',
  enter: '↵',
  tab: '⇥',
  space: '␣',
  arrowup: '↑',
  arrowdown: '↓',
  arrowleft: '←',
  arrowright: '→',
  backspace: '⌫',
  delete: '⌦',
};
const PC_LABELS: Record<string, string> = {
  mod: 'Ctrl',
  shift: 'Shift',
  alt: 'Alt',
  ctrl: 'Ctrl',
  escape: 'Esc',
  enter: 'Enter',
  tab: 'Tab',
  space: 'Space',
  arrowup: '↑',
  arrowdown: '↓',
  arrowleft: '←',
  arrowright: '→',
  backspace: 'Backspace',
  delete: 'Delete',
};

export function formatShortcut(parsed: ParsedShortcut, mac: boolean): string[] {
  const table = mac ? MAC_GLYPHS : PC_LABELS;
  const out: string[] = [];
  if (parsed.ctrl && !parsed.mod) out.push(table.ctrl);
  if (parsed.alt) out.push(table.alt);
  if (parsed.shift) out.push(table.shift);
  if (parsed.mod) out.push(table.mod);
  const k = parsed.key;
  if (table[k]) out.push(table[k]);
  else if (k.length === 1) out.push(k.toUpperCase());
  else if (/^f\d+$/.test(k)) out.push(k.toUpperCase());
  else out.push(k);
  return out;
}
