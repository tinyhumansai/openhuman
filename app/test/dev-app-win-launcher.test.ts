import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const APP_DIR = path.resolve(HERE, '..');
const REPO_ROOT = path.resolve(APP_DIR, '..');
const PACKAGE_JSON_PATH = path.join(APP_DIR, 'package.json');
const LAUNCHER_PATH = path.join(REPO_ROOT, 'scripts', 'run-dev-win.cmd');

type PackageJson = { scripts: Record<string, string> };

const packageJson = JSON.parse(readFileSync(PACKAGE_JSON_PATH, 'utf8')) as PackageJson;
const DEV_APP_WIN = packageJson.scripts['dev:app:win'];

/**
 * The exact `dev:app:win` body that shipped before this regression was fixed.
 * Kept verbatim so the first test proves the failure mode rather than merely
 * asserting the current string.
 */
const LEGACY_BROKEN_SCRIPT = '"C:/Program Files/Git/bin/bash.exe" ../scripts/run-dev-win.sh';

/**
 * Models how `cmd.exe /d /s /c <string>` picks the program to execute, which is
 * how pnpm invokes package.json scripts on Windows.
 *
 * With /S, cmd strips the first and last quote characters of <string> and then
 * parses what remains, taking the first whitespace-delimited token as the
 * program name. That is why an interpreter path containing spaces cannot
 * protect itself with quotes at this layer.
 */
function programTokenUnderCmdS(script: string): string {
  const firstQuote = script.indexOf('"');
  const lastQuote = script.lastIndexOf('"');
  let remaining = script;
  if (firstQuote !== -1 && lastQuote > firstQuote) {
    const head = script.slice(0, firstQuote);
    const middle = script.slice(firstQuote + 1, lastQuote);
    const tail = script.slice(lastQuote + 1);
    remaining = head + middle + tail;
  }
  return remaining.trimStart().split(/\s/)[0];
}

describe('pnpm dev:app:win entry point', () => {
  it('reproduces the historical failure: quotes do not survive cmd.exe /s', () => {
    // cmd sees `C:/Program` as the program name, producing the reported
    // "'C:/Program' is not recognized as an internal or external command".
    expect(programTokenUnderCmdS(LEGACY_BROKEN_SCRIPT)).toBe('C:/Program');
  });

  it('resolves to a single token that cmd.exe can execute verbatim', () => {
    const program = programTokenUnderCmdS(DEV_APP_WIN);

    // If the program token were shorter than the trimmed script body, cmd would
    // have split the path and failed exactly like the legacy form above.
    expect(program).toBe(DEV_APP_WIN.trim());
    expect(program).not.toContain(' ');
    expect(program).not.toContain('"');
  });

  it('points at a launcher that exists in the repository', () => {
    const program = programTokenUnderCmdS(DEV_APP_WIN);
    const resolved = path.resolve(APP_DIR, program.replace(/\\/g, '/'));

    expect(existsSync(resolved), `dev:app:win points at ${resolved}`).toBe(true);
  });

  it('delegates to run-dev-win.sh without trusting a bare bash on PATH', () => {
    const launcher = readFileSync(LAUNCHER_PATH, 'utf8');

    expect(launcher).toContain('run-dev-win.sh');
    // Every bash.exe candidate must be quoted, otherwise the wrapper would
    // reintroduce the space-splitting bug it exists to prevent.
    expect(launcher).toContain('"%BASH_EXE%"');
    // A bare `bash` lookup would hit C:\Windows\System32\bash.exe (the WSL
    // launcher) on machines with WSL enabled.
    expect(launcher).not.toMatch(/where\s+bash\.exe/i);
  });
});
