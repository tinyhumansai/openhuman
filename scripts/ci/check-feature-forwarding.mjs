#!/usr/bin/env node
// Fails when the desktop shell does not forward exactly the gates the product
// is supposed to ship.
//
// See scripts/lib/feature-forwarding.mjs for the three assertions and why they
// are shaped this way (#4919). Short version: the shell sets
// `default-features = false` on `openhuman-rpc` (its only openhuman
// dependency, which forwards each gate down to the core), so every gate the product
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
// tinyhumans, rpc, then the cli and tui hosts on rpc (#6364). Those lists were maintained by hand: a gate
// dropped from the core and left behind is a cargo error nobody reads as drift
// (#6360), and a gate ADDED to the core and forgotten is silent, because the
// product lanes only ever resolve names against `openhuman-cli`.
//
// Usage: check-feature-forwarding.mjs [core-manifest] [shell-manifest] [product-features]
//                                     [embed-manifest] [tinyhumans-manifest] [cli-manifest]
//                                     [rpc-manifest] [tui-manifest]
import { readFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

import {
  CHAIN_GATES_NOT_FORWARDED,
  CHAIN_LOCAL_GATES,
  checkLinkedModulesIsolation,
  LINKED_MODULES_FEATURE,
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
  rpcForwardedGates,
} from '../lib/feature-forwarding.mjs';

const REPO_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../..');

function usage() {
  return (
    'Usage: check-feature-forwarding.mjs [core-manifest] [shell-manifest] [product-features]\n' +
    '                                    [embed-manifest] [tinyhumans-manifest] [cli-manifest]\n' +
    '                                    [rpc-manifest] [tui-manifest]'
  );
}

const [coreArg, shellArg, productArg, embedArg, tinyhumansArg, cliArg, rpcArg, tuiArg, extra] =
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
const rpcPath = rpcArg ? resolve(rpcArg) : resolve(REPO_ROOT, 'crates/openhuman-rpc/Cargo.toml');
const tuiPath = tuiArg ? resolve(tuiArg) : resolve(REPO_ROOT, 'crates/openhuman-tui/Cargo.toml');

let coreToml;
let shellToml;
let productText;
let embedToml;
let tinyhumansToml;
let cliToml;
let rpcToml;
let tuiToml;
try {
  coreToml = readFileSync(corePath, 'utf8');
  shellToml = readFileSync(shellPath, 'utf8');
  productText = readFileSync(productPath, 'utf8');
  embedToml = readFileSync(embedPath, 'utf8');
  tinyhumansToml = readFileSync(tinyhumansPath, 'utf8');
  cliToml = readFileSync(cliPath, 'utf8');
  rpcToml = readFileSync(rpcPath, 'utf8');
  tuiToml = readFileSync(tuiPath, 'utf8');
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
const product = checkProductForwarding({ productFeatures, coreFeatureNames, hostFeatureNames: parseCoreFeatureNames(rpcToml), shell });
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
// embed, then tinyhumans, then rpc (and cli), and each hop can drop one.
const embedFeatures = parseFeatureTable(embedToml);
const tinyhumansFeatures = parseFeatureTable(tinyhumansToml);
const cliFeatures = parseFeatureTable(cliToml);
const rpcFeatures = parseFeatureTable(rpcToml);
const tuiFeatures = parseFeatureTable(tuiToml);

// Guard the guard, same as above: a parser that found nothing would turn every
// chain assertion into a rubber stamp.
for (const [path, table] of [
  [embedPath, embedFeatures],
  [tinyhumansPath, tinyhumansFeatures],
  [cliPath, cliFeatures],
  [rpcPath, rpcFeatures],
  [tuiPath, tuiFeatures],
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
// What a host must forward from rpc: its gates minus rpc's own local ones.
const rpcHostGates = rpcForwardedGates(rpcFeatures);

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
    // rpc sits on tinyhumans alone and forwards every one of its gates
    // (product gates plus `jev`); its server/client/store gates are local.
    crate: 'openhuman-rpc',
    features: rpcFeatures,
    sources: [{ crate: 'openhuman-tinyhumans', gates: tinyhumansGates, required: true }],
  },
  {
    // The hosts sit on rpc alone (no core/tinyhumans edge any more).
    crate: 'openhuman-cli',
    features: cliFeatures,
    sources: [{ crate: 'openhuman-rpc', gates: rpcHostGates, required: true }],
  },
  {
    crate: 'openhuman-tui',
    features: tuiFeatures,
    sources: [{ crate: 'openhuman-rpc', gates: rpcHostGates, required: true }],
  },
].map(link =>
  diffChainForwarding({
    ...link,
    notForwarded: CHAIN_GATES_NOT_FORWARDED[link.crate] ?? {},
    localGates: CHAIN_LOCAL_GATES[link.crate] ?? {},
  })
);

console.log('');
console.log('Library chain (core -> embed -> tinyhumans -> rpc -> cli/tui; app on its rpc dependency):');
for (const result of chain) {
  console.log(
    formatChainReport(result, {
      notForwarded: CHAIN_GATES_NOT_FORWARDED[result.crate] ?? {},
    })
  );
}

const chainOk = chain.every(result => result.ok);

// The local-development linked-module gate must stay out of every shipped path.
const linkedViolations = checkLinkedModulesIsolation({ productFeatures, shellToml, cliFeatures });
console.log('');
if (linkedViolations.length === 0) {
  console.log(`${LINKED_MODULES_FEATURE} isolation: OK (not a product gate, not in the app manifest, not a CLI default).`);
} else {
  console.log(`${LINKED_MODULES_FEATURE} isolation: FAIL`);
  for (const violation of linkedViolations) console.log(`  - ${violation}`);
}

process.exit(product.ok && defaults.ok && chainOk && linkedViolations.length === 0 ? 0 : 1);
