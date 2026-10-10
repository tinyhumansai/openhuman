#!/usr/bin/env node
import { spawnSync } from "node:child_process";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..");
export function discoverExamples(directory = resolve(root, "crates/openhuman-embed/examples")) {
  return readdirSync(directory).filter((file) => file.endsWith(".rs")).sort().map((file) => {
    const source = readFileSync(resolve(directory, file), "utf8");
    const feature = source.match(/^\/\/! Feature: (.+)$/m)?.[1]?.trim();
    if (!source.includes("//! Title:") || !source.includes("//! Run:") || !source.includes("// ANCHOR:")) {
      throw new Error(`${file}: missing example metadata or documentation anchor`);
    }
    const profile = source.match(/^\/\/! Profile: (.+)$/m)?.[1]?.trim() ?? "dev";
    const defaultFeatures = source.match(/^\/\/! Default features: (.+)$/m)?.[1]?.trim() !== "disabled";
    return { name: file.slice(0, -3), features: feature && feature !== "default" ? [feature] : [], profile, defaultFeatures };
  });
}
export function offlineEnvironment(environment) {
  return Object.fromEntries(Object.entries(environment).filter(([name]) =>
    !name.startsWith("OPENHUMAN_EXAMPLE_") && !name.startsWith("OPENHUMAN_BACKEND_") &&
    !name.startsWith("OPENHUMAN_KEYRING_") &&
    name !== "OPENHUMAN_STORAGE_URL" && name !== "OPENHUMAN_WORKSPACE"));
}
export function assertExampleOutput(name, result) {
  if (result.error) throw result.error;
  if (result.status !== 0) throw new Error(`${name} exited ${result.status}\n${result.stdout}\n${result.stderr}`);
  if (!result.stdout.split(/\r?\n/).includes(`EXAMPLE_OK ${name}`)) {
    throw new Error(`${name}: successful exit without behavioral assertion marker\n${result.stdout}`);
  }
}
export function runExamples({ run = spawnSync, examples = discoverExamples(), environment = process.env } = {}) {
  // Share one feature graph per declared build configuration. Measurements
  // retain their release/minimal graph rather than inheriting other examples.
  const configuration = (example) => `${example.profile ?? "dev"}:${example.defaultFeatures !== false}`;
  const featureGroups = new Map();
  for (const example of examples) {
    const key = configuration(example);
    const features = featureGroups.get(key) ?? new Set();
    for (const feature of example.features) features.add(feature);
    featureGroups.set(key, features);
  }
  for (const example of examples) {
    const args = ["run", "--quiet", "-p", "openhuman-embed", "--example", example.name];
    if (example.profile === "release") args.push("--release");
    else if (example.profile && example.profile !== "dev") args.push("--profile", example.profile);
    if (example.defaultFeatures === false) args.push("--no-default-features");
    const features = [...featureGroups.get(configuration(example))].sort();
    if (features.length) args.push("--features", features.join(","));
    const result = run(resolve(root, "scripts/ci-cancel-aware.sh"), ["cargo", ...args], {
      cwd: root, encoding: "utf8", env: offlineEnvironment(environment), timeout: 1800000, maxBuffer: 16 * 1024 * 1024,
    });
    assertExampleOutput(example.name, result);
    console.log(`PASS ${example.name}`);
  }
}
if (process.argv[1] && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  if (process.argv.includes("--list")) console.log(discoverExamples().map((example) => example.name).join("\n"));
  else runExamples();
}
