# scripts/test-planning

Mine GitHub issues and PRs into a test-planning backlog.

## What it does

1. Fetches repository issues and pull requests over GitHub GraphQL into JSONL.
2. Chunks those sources and asks an LLM CLI to compress them into concrete test
   backlog items.
3. Emits:
   - `raw-issues.jsonl`, `raw-prs.jsonl`, `raw-all.jsonl`
   - `synthesized-raw.jsonl`
   - `test-map.jsonl`
   - `test-map.md`

## Usage

```sh
# Full run against the default repo
pnpm test:plan

# Smaller sample
pnpm test:plan -- \
  --max-issues 25 \
  --max-prs 25 \
  --chunk-size 10

# Resume from a fetched corpus
pnpm test:plan -- \
  --phase synthesize \
  --out-dir tmp/test-planning/20260514T120000Z
```

## Notes

- Default repo is `tinyhumansai/openhuman`.
- Requires `gh auth status` to be healthy.
- Default synthesizer is `codex`; `--llm claude` is also supported.
- The Markdown output is intentionally compressed. The JSONL is the better input
  if you want to do a second dedupe or planning pass later.
