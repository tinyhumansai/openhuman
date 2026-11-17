import { readFileSync } from 'node:fs';
import { resolve } from 'node:path';
import { describe, expect, it } from 'vitest';

const configPath = resolve(process.cwd(), '../crates/openhuman-app/tauri.conf.json');
const config = JSON.parse(readFileSync(configPath, 'utf8')) as {
  app?: { security?: { csp?: string } };
};
const scriptSourceTokens =
  config.app?.security?.csp
    ?.split(';')
    .map(directive => directive.trim())
    .find(directive => directive.startsWith('script-src '))
    ?.split(/\s+/)
    .slice(1) ?? [];

const connectSourceTokens =
  config.app?.security?.csp
    ?.split(';')
    .map(directive => directive.trim())
    .find(directive => directive.startsWith('connect-src '))
    ?.split(/\s+/)
    .slice(1) ?? [];

describe('Tauri content security policy', () => {
  it('allows scripts served from the Wry custom scheme', () => {
    expect(scriptSourceTokens).toEqual(expect.arrayContaining(['tauri:', 'tauri://localhost']));
  });

  it('retains the existing script execution requirements', () => {
    expect(scriptSourceTokens).toEqual(
      expect.arrayContaining(["'self'", "'wasm-unsafe-eval'", 'https://www.googletagmanager.com'])
    );
  });

  it('limits plain WebSockets to loopback and keeps secure remote WebSockets', () => {
    expect(connectSourceTokens).toEqual(
      expect.arrayContaining(['ws://127.0.0.1:*', 'ws://localhost:*', 'wss:'])
    );
    expect(connectSourceTokens).not.toContain('ws:');
  });
});
