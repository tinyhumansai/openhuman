import { existsSync, readFileSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { describe, expect, it } from 'vitest';

const HERE = path.dirname(fileURLToPath(import.meta.url));
const APP_DIR = path.resolve(HERE, '..');
const REPO_ROOT = path.resolve(APP_DIR, '..');

const packageJson = JSON.parse(
  readFileSync(path.join(APP_DIR, 'package.json'), 'utf8')
) as { scripts: Record<string, string> };

const DEV_APP_WIN = packageJson.scripts['dev:app:win'];

/**
 * The exact `dev:app:win` body that shipped before this regression was fixed.
 * Kept verbatim so the test below proves the failure mode rather than just
 * asserting the current string.
 */
const LEGACY_BROKEN_SCRIPT = '"C:/Program Files/Git/bin/bash.exe" ../scripts/run-dev-win.sh';

/**
 * Models how `cmd.exe /d /s /c <string>` picks the program to execute, which
 * is how pnpm invokes package.json scripts on Windows.
 *
 * With /S, cmd strips the first and last quote characters of <string> and
 * parses what remains, taking the first whitespace-delimited token as the
 * program name. That is why an interpreter path containing spaces cannot
 * protect itself with quotes at this layer.
 */
function programTokenUnderCmdS(script: string): string {
  let remaining = script;
  const firstQuote = remaining.indexOf('"');
  const lastQuote = remaining.lastIndexOf('"');
  if (firstQuote !== -1 && lastQuote > firstQuote) {
    remaining =
      remaining.slice(0, firstQuote) +
      remaining.slice(firstQuote + 1, lastQuote) +
      remaining.slice(lastQuote + 1);
  }
  return remaining.trimStart().split(/\s/)[0] ?? '';
}

describe('pnpm dev:app:win entry point', () => {
  it('reproduces the historical failure: quotes do not survive cmd.exe /s', () => {
    // Documents the bug this script layout fixes. cmd sees `C:/Program` as the
    // program name, producing "'C:/Program' is not recognized...".
    expect(programTokenUnderCmdS(LEGACY_BROKEN_SCRIPT)).toBe('C:/Program');
  });

  it('resolves to a single token that cmd.exe can execute verbatim', () => {
    const program = programTokenUnderCmdS(DEV_APP_WIN);

    // The whole script must survive cmd's quote stripping intact: if the
    // program token is shorter than the trimmed script body, cmd would have
    // split the path and failed exactly like the legacy form above.
    expect(program).toBe(DEV_APP_WIN.trim());
    expect(program).not.toContain(' ');
    expect(program).not.toContain('"');
  });

  it('points at a launcher that exists in the repository', () => {
    const program = programTokenUnderCmdS(DEV_APP_WIN);
    const resolved = path.resolve(APP_DIR, program.replace(/\\/g, '/'));

    expect(
      existsSync(resolved),
      `dev:app:win points at ${program}, which does not exist at ${resolved}`
    ).toBe(true);
  });

  it('delegates to run-dev-win.sh without trusting a bare bash on PATH', () => {
    const launcher = readFileSync(
      path.join(REPO_ROOT, 'scripts', 'run-dev-win.cmd'),
      'utf8'
    );

    expect(launcher).toContain('run-dev-win.sh');
    // Every bash.exe candidate must be quoted, otherwise the wrapper would
    // reintroduce the space-splitting bug it exists to prevent.
    expect(launcher).toContain('"%BASH_EXE%"');
    // A bare `bash` lookup would hit C:\Windows\System32\bash.exe (the WSL
    // launcher) on machines with WSL enabled.
    expect(launcher).not.toMatch(/where\s+bash\.exe/i);
  });
});
