#!/usr/bin/env node

import fs from "node:fs";
import path from "node:path";

const ROOT = "src";
const FEATURE_GATE =
  /#\[cfg\((?:not\()?feature = "(?:voice|media|web3|meet|mcp|skills|flows|channels|contacts)"|#\[cfg\((?:not\()?all\([^\]]*feature = "contacts"/;
const TEST_MARKER = /#\[test\]|#\[tokio::test\]|fn .*_test/;
const PATH_MODULE =
  /#\[path\s*=\s*"([^"]+)"\]\s*(?:#\[[^\]]+\]\s*)*(?:pub(?:\([^)]*\))?\s+)?mod\s+\w+\s*;/g;

function rustFiles(directory) {
  return fs.readdirSync(directory, { withFileTypes: true }).flatMap((entry) => {
    const file = path.join(directory, entry.name);
    if (entry.isDirectory()) return rustFiles(file);
    return entry.isFile() && entry.name.endsWith(".rs") ? [file] : [];
  });
}

function moduleContainsTest(file, seen = new Set()) {
  const absolute = path.resolve(file);
  if (seen.has(absolute) || !fs.existsSync(absolute)) return false;
  seen.add(absolute);

  const source = fs.readFileSync(absolute, "utf8");
  if (TEST_MARKER.test(source)) return true;

  for (const match of source.matchAll(PATH_MODULE)) {
    const child = path.resolve(path.dirname(absolute), match[1]);
    if (moduleContainsTest(child, seen)) return true;
  }
  return false;
}

for (const file of rustFiles(ROOT)) {
  const source = fs.readFileSync(file, "utf8");
  if (FEATURE_GATE.test(source) && moduleContainsTest(file)) {
    console.log(path.relative(ROOT, file).split(path.sep).join("/"));
  }
}
