#!/usr/bin/env node
import { parseChecklist, summarize } from './lib/checklist-parser.mjs';

const body = process.env.PR_BODY ?? '';
const parsed = parseChecklist(body);
console.log(summarize(parsed));
if (parsed.totalUnchecked > 0) {
  process.exit(1);
}
