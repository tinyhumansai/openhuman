#!/usr/bin/env node
// Gate side of the sharded hosted `rust-cov` lane.
//
// Usage: merge-coverage-shards.mjs <artifacts-dir> <out.info> [--matrix <json>]
//   <artifacts-dir>  where the `ci-out-*` artifacts were downloaded
//   <out.info>       the merged lcov of every shard, for the whole-tree
//                    coverage-presence check
//   --matrix <json>  the plan job's matrix output (default: $PLAN_MATRIX)
//
// Every rust-cov shard (RUST_COV_SHARDS) must be in the job matrix and must
// have uploaded a lcov with source records. A shard that did not run, did not
// finish, or measured nothing is an error here, so a missing report can never
// read as "nothing to cover". diff-cover reads the same per-shard files.
import { existsSync, readFileSync, writeFileSync } from "node:fs";
import path from "node:path";
import { pathToFileURL } from "node:url";

import { RUST_COV_SHARDS } from "./lanes-plan.mjs";

export const SHARD_LCOV = "lcov/lcov-core.info";

/**
 * @param {object} opts
 * @param {{include?: {group: string}[]}} opts.matrix  the plan's job matrix
 * @param {(group: string) => string|null} opts.read   a shard's lcov text, or
 *   null when its artifact or file is absent
 * @param {boolean} [opts.required]  false when the Rust core was not tested
 * @returns {{errors: string[], merged: string}}
 */
export function checkShards({ matrix, read, required = true }) {
  const errors = [];
  const inMatrix = new Set((matrix?.include ?? []).map((g) => g.group));
  const parts = [];
  for (const shard of RUST_COV_SHARDS) {
    const group = `rust-cov-${shard}`;
    if (!required) continue;
    if (!inMatrix.has(group)) {
      errors.push(`${group} is not in the job matrix; its coverage was dropped`);
      continue;
    }
    const text = read(group);
    if (text === null || text === undefined) {
      errors.push(
        `${group} has no lcov (${SHARD_LCOV} in artifact ci-out-${group}); the shard did not finish`,
      );
    } else if (!/^SF:/m.test(text)) {
      errors.push(`${group} produced no source records in its lcov`);
    } else {
      parts.push(text.endsWith("\n") ? text : `${text}\n`);
    }
  }
  return { errors, merged: parts.join("") };
}

function main(argv) {
  const [dir, out, ...rest] = argv;
  if (!dir || !out) {
    console.error("usage: merge-coverage-shards.mjs <artifacts-dir> <out.info> [--matrix <json>]");
    return 2;
  }
  const flag = rest.indexOf("--matrix");
  const raw = flag >= 0 ? rest[flag + 1] : process.env.PLAN_MATRIX;
  let matrix;
  try {
    matrix = JSON.parse(raw ?? "");
  } catch {
    console.error("::error::merge-coverage-shards: the plan matrix is missing or not JSON");
    return 2;
  }
  const { errors, merged } = checkShards({
    matrix,
    read: (group) => {
      const file = path.join(dir, `ci-out-${group}`, SHARD_LCOV);
      return existsSync(file) ? readFileSync(file, "utf8") : null;
    },
  });
  for (const e of errors) console.error(`::error::[ci][rust-cov] ${e}`);
  if (errors.length > 0) return 1;
  writeFileSync(out, merged);
  console.log(
    `[ci][rust-cov] merged ${RUST_COV_SHARDS.length} shard reports into ${out} (${merged.split("\n").length} lines)`,
  );
  return 0;
}

if (import.meta.url === pathToFileURL(process.argv[1] ?? "").href) {
  process.exit(main(process.argv.slice(2)));
}
