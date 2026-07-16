#!/usr/bin/env node
// Fails when the desktop shell does not forward a default-ON core Cargo gate.
//
// See scripts/lib/feature-forwarding.mjs for why this exists (#4919). Short
// version: the shell sets `default-features = false` on `openhuman_core`, so
// every default-ON gate must be forwarded by hand. When someone forgets, the
// domain vanishes from the shipped app with no build error — that is how #4901
// (voice, 56 users) and #4918 (tokenjuice-treesitter) shipped.
//
// Usage: check-feature-forwarding.mjs [core-manifest] [shell-manifest]
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  diffForwarding,
  formatReport,
  parseCoreDefaultFeatures,
  parseShellForwardedFeatures,
} from '../lib/feature-forwarding.mjs';

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

/**
 * Gates the desktop shell intentionally does NOT forward, mapped to why.
 *
 * Empty by design: every current default-ON gate belongs in the shipped app.
 * Adding an entry is a deliberate product decision, not a way to silence this
 * check — the reason string is what a future reader (and reviewer) relies on to
 * tell "excluded on purpose" from "forgotten". That ambiguity is exactly what
 * let #4918 sit unnoticed since #4123.
 */
const INTENTIONALLY_NOT_FORWARDED = {
  // 'some-gate': 'Reason it must not ship in the desktop build.',
};

function usage() {
  return 'Usage: check-feature-forwarding.mjs [core-manifest] [shell-manifest]';
}

const [coreArg, shellArg, extra] = process.argv.slice(2);
if (coreArg === '--help' || coreArg === '-h') {
  console.log(usage());
  process.exit(0);
}
if (extra) {
  console.error(usage());
  process.exit(2);
}

const corePath = coreArg ? resolve(coreArg) : resolve(REPO_ROOT, 'Cargo.toml');
const shellPath = shellArg ? resolve(shellArg) : resolve(REPO_ROOT, 'app/src-tauri/Cargo.toml');

let coreToml;
let shellToml;
try {
  coreToml = readFileSync(corePath, 'utf8');
  shellToml = readFileSync(shellPath, 'utf8');
} catch (err) {
  console.error(`Could not read manifests: ${err.message}`);
  process.exit(2);
}

const coreDefaults = parseCoreDefaultFeatures(coreToml);
const shell = parseShellForwardedFeatures(shellToml);

// A parser that silently finds nothing would turn this guard into a rubber
// stamp, which is worse than not having it. Treat "no defaults found" as a
// failure of the check itself rather than a pass.
if (coreDefaults.length === 0) {
  console.error(
    `FAIL: parsed zero default features from ${corePath}.\n` +
      'Either the manifest changed shape or the parser is broken — refusing to pass vacuously.'
  );
  process.exit(2);
}

const result = diffForwarding({ coreDefaults, shell, allowlist: INTENTIONALLY_NOT_FORWARDED });
console.log(formatReport(result, { coreDefaults, shell, allowlist: INTENTIONALLY_NOT_FORWARDED }));
process.exit(result.ok ? 0 : 1);
