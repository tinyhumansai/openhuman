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
// It also checks the library chain the core is re-declared by — embed,
// tinyhumans and cli (#6364). Those three lists were maintained by hand: a gate
// dropped from the core and left behind is a cargo error nobody reads as drift
// (#6360), and a gate ADDED to the core and forgotten is silent, because the
// product lanes only ever resolve names against `openhuman-cli`.
//
// Usage: check-feature-forwarding.mjs [core-manifest] [shell-manifest] [product-features]
//                                     [embed-manifest] [tinyhumans-manifest] [cli-manifest]
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  CHAIN_GATES_NOT_FORWARDED,
  CHAIN_LOCAL_GATES,
  checkProductForwarding,
  diffChainForwarding,
  diffForwarding,
  formatChainReport,
  formatProductReport,
  formatReport,
  INTENTIONALLY_NOT_FORWARDED,
  parseCoreDefaultFeatures,
  parseCoreFeatureNames,
  parseFeatureTable,
  parseProductFeatures,
  parseShellForwardedFeatures,
} from '../lib/feature-forwarding.mjs';

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

function usage() {
  return (
    'Usage: check-feature-forwarding.mjs [core-manifest] [shell-manifest] [product-features]\n' +
    '                                    [embed-manifest] [tinyhumans-manifest] [cli-manifest]'
  );
}

const [coreArg, shellArg, productArg, embedArg, tinyhumansArg, cliArg, extra] =
  process.argv.slice(2);
if (coreArg === '--help' || coreArg === '-h') {
  console.log(usage());
  process.exit(0);
}
if (extra) {
  console.error(usage());
  process.exit(2);
}

const corePath = coreArg
  ? resolve(coreArg)
  : resolve(REPO_ROOT, 'crates/openhuman-core/Cargo.toml');
const shellPath = shellArg ? resolve(shellArg) : resolve(REPO_ROOT, 'crates/openhuman-app/Cargo.toml');
const productPath = productArg
  ? resolve(productArg)
  : resolve(REPO_ROOT, 'scripts/ci/product-features.txt');
const embedPath = embedArg
  ? resolve(embedArg)
  : resolve(REPO_ROOT, 'crates/openhuman-embed/Cargo.toml');
const tinyhumansPath = tinyhumansArg
  ? resolve(tinyhumansArg)
  : resolve(REPO_ROOT, 'crates/openhuman-tinyhumans/Cargo.toml');
const cliPath = cliArg ? resolve(cliArg) : resolve(REPO_ROOT, 'crates/openhuman-cli/Cargo.toml');

let coreToml;
let shellToml;
let productText;
let embedToml;
let tinyhumansToml;
let cliToml;
try {
  coreToml = readFileSync(corePath, 'utf8');
  shellToml = readFileSync(shellPath, 'utf8');
  productText = readFileSync(productPath, 'utf8');
  embedToml = readFileSync(embedPath, 'utf8');
  tinyhumansToml = readFileSync(tinyhumansPath, 'utf8');
  cliToml = readFileSync(cliPath, 'utf8');
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

// Assertion 4: the library chain (#6364). The core's gates are re-declared by
// embed, then tinyhumans, then cli, and each hop can drop one.
const embedFeatures = parseFeatureTable(embedToml);
const tinyhumansFeatures = parseFeatureTable(tinyhumansToml);
const cliFeatures = parseFeatureTable(cliToml);

// Guard the guard, same as above: a parser that found nothing would turn every
// chain assertion into a rubber stamp.
for (const [path, table] of [
  [embedPath, embedFeatures],
  [tinyhumansPath, tinyhumansFeatures],
  [cliPath, cliFeatures],
]) {
  if (table.size === 0) {
    console.error(
      `FAIL: parsed zero features from ${path}.\n` +
        'Either the manifest changed shape or the parser is broken — refusing to pass vacuously.'
    );
    process.exit(2);
  }
}

const embedGates = [...embedFeatures.keys()].filter(name => name !== 'default');
const tinyhumansGates = [...tinyhumansFeatures.keys()].filter(name => name !== 'default');

const chain = [
  {
    crate: 'openhuman-embed',
    features: embedFeatures,
    sources: [{ crate: 'openhuman-core', gates: coreFeatureNames, required: true }],
  },
  {
    crate: 'openhuman-tinyhumans',
    features: tinyhumansFeatures,
    sources: [{ crate: 'openhuman-embed', gates: embedGates, required: true }],
  },
  {
    crate: 'openhuman-cli',
    features: cliFeatures,
    sources: [
      { crate: 'openhuman-core', gates: coreFeatureNames, required: true },
      // Optional: the cli forwards to both parents, but only for the gates
      // tinyhumans actually has — `e2e-test-support` is core-only.
      { crate: 'openhuman-tinyhumans', gates: tinyhumansGates, required: false },
    ],
  },
].map(link =>
  diffChainForwarding({
    ...link,
    notForwarded: CHAIN_GATES_NOT_FORWARDED[link.crate] ?? {},
    localGates: CHAIN_LOCAL_GATES[link.crate] ?? {},
  })
);

console.log('');
console.log('Library chain (core -> embed -> tinyhumans -> cli):');
for (const result of chain) {
  console.log(
    formatChainReport(result, {
      notForwarded: CHAIN_GATES_NOT_FORWARDED[result.crate] ?? {},
    })
  );
}

const chainOk = chain.every(result => result.ok);

process.exit(product.ok && defaults.ok && chainOk ? 0 : 1);
