#!/usr/bin/env node
import { readFileSync } from 'node:fs';
import { parseChecklist, summarize } from './lib/checklist-parser.mjs';

function readBody() {
  const [source, extra] = process.argv.slice(2);
  if (extra) {
    console.error('Usage: check-pr-checklist.mjs [body-file|-]');
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
