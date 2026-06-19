import { readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const TAURI_ROOT = path.resolve(HERE, '..', 'src-tauri');
const DEFAULT_CAPABILITY_PATH = path.join(TAURI_ROOT, 'capabilities', 'default.json');
const PTT_PERMISSION_PATH = path.join(TAURI_ROOT, 'permissions', 'allow-ptt-hotkey-control.toml');

const PTT_PERMISSION = 'allow-ptt-hotkey-control';
const PTT_COMMANDS = ['register_ptt_hotkey', 'unregister_ptt_hotkey'] as const;

function extractAllowedCommands(toml: string): string[] {
  const allowBlock = toml.match(/allow\s*=\s*\[(?<body>[\s\S]*?)\]/)?.groups?.body;
  if (!allowBlock) {
    throw new Error('permission TOML is missing an allow = [...] command list');
  }

  return Array.from(allowBlock.matchAll(/"([^"]+)"/g), match => match[1]);
}

describe('desktop Tauri ACL for push-to-talk hotkeys', () => {
  const defaultCapability = JSON.parse(readFileSync(DEFAULT_CAPABILITY_PATH, 'utf8')) as {
    permissions?: unknown[];
  };
  const permissionToml = readFileSync(PTT_PERMISSION_PATH, 'utf8');

  it('grants the PTT hotkey permission to the default desktop capability', () => {
    expect(defaultCapability.permissions).toContain(PTT_PERMISSION);
  });

  it.each(PTT_COMMANDS)('allows the %s command through that permission', command => {
    expect(extractAllowedCommands(permissionToml)).toContain(command);
  });
});
