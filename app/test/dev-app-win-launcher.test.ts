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
 * A batch label, optionally behind the `@` echo-suppression prefix.
 */
const BATCH_LABEL = /^@?:/;

/**
 * A `goto`/`call` jump in any form cmd accepts: optionally prefixed with `@`,
 * introduced by start-of-line or a command separator, and followed by either
 * whitespace (`goto :eof`) or a colon (`goto:eof`).
 */
const BATCH_JUMP = /(^|[\s&|()])@?(goto|call)[\s:]/i;

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

/** Executable lines of the launcher: comments and blank lines removed. */
function launcherCodeLines(launcher: string): string[] {
  return launcher
    .split('\n')
    .map(line => line.trim())
    .filter(line => line.length > 0)
    .filter(line => !line.toLowerCase().startsWith('rem'));
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

  it('recognises every batch jump form, so the guard below cannot be bypassed', () => {
    // Positive controls: each of these must count as a jump.
    for (const jump of [
      'goto :label',
      'call :label',
      '@goto :label',
      '@call :label',
      'goto:eof',
      '@goto:eof',
      '& goto :label',
      '( @goto :label)',
      'if not defined BASH_EXE goto :no_bash',
    ]) {
      expect(BATCH_JUMP.test(jump), jump).toBe(true);
    }

    // Negative controls: ordinary text must not trip the guard.
    for (const plain of [
      'echo go to label',
      'echo recall the value',
      'exit /b %ERRORLEVEL%',
      '"%BASH_EXE%" "%SCRIPT_DIR%run-dev-win.sh" %*',
    ]) {
      expect(BATCH_JUMP.test(plain), plain).toBe(false);
    }
  });

  it('uses no label-based control flow, so line endings cannot change behaviour', () => {
    const code = launcherCodeLines(readFileSync(LAUNCHER_PATH, 'utf8'));

    // cmd.exe finds a label by byte offset and re-reads the script in 512-byte
    // chunks, so `goto`/`call :label` is the one construct whose behaviour
    // depends on whether the file is LF or CRLF. Keeping the launcher free of
    // labels makes it correct either way instead of relying on a checkout rule.
    expect(code.filter(line => BATCH_LABEL.test(line))).toEqual([]);
    expect(code.filter(line => BATCH_JUMP.test(line))).toEqual([]);
    expect(code.length).toBeGreaterThan(0);
  });
});
