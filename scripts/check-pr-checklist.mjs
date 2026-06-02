#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import { parseChecklist, summarize } from './lib/checklist-parser.mjs';

function usage() {
  return 'Usage: check-pr-checklist.mjs [body-file|-]';
}

function readBody() {
  const [source, extra] = process.argv.slice(2);
  if (source === '--help' || source === '-h') {
    console.log(usage());
    process.exit(0);
  }
  if (extra) {
    console.error(usage());
    process.exit(2);
  }
  if (source === '-') {
    return readFileSync(0, 'utf8');
  }
  if (source) {
    return readFileSync(source, 'utf8');
  }
  return process.env.PR_BODY ?? '';
}

const body = readBody();
const parsed = parseChecklist(body);
console.log(summarize(parsed));
if (parsed.totalUnchecked > 0) {
  process.exit(1);
}
