#!/usr/bin/env node
// Fails when the desktop shell does not forward exactly the gates the product
// is supposed to ship.
//
// See scripts/lib/feature-forwarding.mjs for the three assertions and why they
// are shaped this way (#4919). Short version: the shell sets
// `default-features = false` on `openhuman_core`, so every gate the product
// needs must be forwarded by hand. When someone forgets, the domain vanishes
// from the shipped app with no build error — that is how #4901 (voice, 56
// users, ~93k Sentry events) and #4918 (tokenjuice-treesitter, silent soft
// degradation) shipped.
//
// The product set lives in scripts/ci/product-features.txt, NOT in
// `[features] default` — `default` is the contributor set now and is
// deliberately smaller.
//
// Usage: check-feature-forwarding.mjs [core-manifest] [shell-manifest] [product-features]
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  checkProductForwarding,
  diffForwarding,
  formatProductReport,
  formatReport,
  INTENTIONALLY_NOT_FORWARDED,
  parseCoreDefaultFeatures,
  parseCoreFeatureNames,
  parseProductFeatures,
  parseShellForwardedFeatures,
} from '../lib/feature-forwarding.mjs';

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

function usage() {
  return 'Usage: check-feature-forwarding.mjs [core-manifest] [shell-manifest] [product-features]';
}

const [coreArg, shellArg, productArg, extra] = process.argv.slice(2);
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
const productPath = productArg
  ? resolve(productArg)
  : resolve(REPO_ROOT, 'scripts/ci/product-features.txt');

let coreToml;
let shellToml;
let productText;
try {
  coreToml = readFileSync(corePath, 'utf8');
  shellToml = readFileSync(shellPath, 'utf8');
  productText = readFileSync(productPath, 'utf8');
} catch (err) {
  console.error(`Could not read inputs: ${err.message}`);
  process.exit(2);
}

const coreDefaults = parseCoreDefaultFeatures(coreToml);
const coreFeatureNames = parseCoreFeatureNames(coreToml);
const productFeatures = parseProductFeatures(productText);
const shell = parseShellForwardedFeatures(shellToml);

// Guard the guard. A parser that silently found nothing would turn this into a
// rubber stamp, which is worse than having no check at all — so treat empty
// input as a failure OF THE CHECK (exit 2), distinct from a real drift (exit 1).
//
// `coreDefaults` is deliberately NOT in this list: an empty `default` is a
// legitimate configuration (a core where every gate is opt-in), and assertion 1
// does not depend on it. That is the whole point of the product-set rewrite.
if (productFeatures.length === 0) {
  console.error(
    `FAIL: parsed zero product gates from ${productPath}.\n` +
      'Either the file changed shape or the parser is broken — refusing to pass vacuously.'
  );
  process.exit(2);
}
if (coreFeatureNames.length === 0) {
  console.error(
    `FAIL: parsed zero feature names from ${corePath}.\n` +
      'Either the manifest changed shape or the parser is broken — refusing to pass vacuously.'
  );
  process.exit(2);
}

// Assertions 1 + 2.
const product = checkProductForwarding({ productFeatures, coreFeatureNames, shell });
console.log(formatProductReport(product, { productFeatures, shell }));

// Assertion 3. Still worth running: it is what catches a gate added to
// `default` (so contributors get it) that nobody remembered to also ship.
const defaults = diffForwarding({
  coreDefaults,
  shell,
  allowlist: INTENTIONALLY_NOT_FORWARDED,
});
console.log('');
console.log(formatReport(defaults, { coreDefaults, shell, allowlist: INTENTIONALLY_NOT_FORWARDED }));

process.exit(product.ok && defaults.ok ? 0 : 1);
