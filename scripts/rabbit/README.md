# scripts/rabbit

Auto-retrigger [CodeRabbit](https://coderabbit.ai) reviews on PRs whose rate-limit window has elapsed.

CodeRabbit (Pro) reviews **5 PRs/hr** per developer. When you push a flurry of
commits across several PRs, CR posts a "Rate limit exceeded, please wait
N minutes" comment instead of reviewing. Once the wait elapses you have to
manually comment `@coderabbitai review` on each PR. This script does that pass
for you.

## Usage

```sh
pnpm rabbit                # default: scan + retrigger up to 5 PRs
pnpm rabbit list           # report-only; no comments posted
pnpm rabbit run --dry-run  # show what would be retriggered
pnpm rabbit run --max 3    # cap retriggers this run
pnpm rabbit run --pr 1409  # one PR only
pnpm rabbit run --grace 60 # wait 60s past CR's stated time before retriggering
```

Pair with `/loop` to drain a backlog automatically:

```
/loop 15m pnpm rabbit run --max 5
```

## How it works

For each open PR:

1. Pull `issues/<pr>/comments`, find CodeRabbit's latest comment.
2. If it carries the marker `<!-- rate limited by coderabbit.ai -->`, parse the
   stated wait (`Please wait **46 seconds**…`).
3. Skip if CR has posted a non-rate-limit comment since (it recovered) or if
   anyone has already commented `@coderabbitai review` after the rate-limit
   notice (in flight).
4. If `created_at + wait + grace` is in the past, post `@coderabbitai review`.

## Config

- `RABBIT_REPO=owner/name`, override target repo (default: `upstream` remote).
- Requires `gh` and `node`.

## See also

- [`scripts/shortcuts/review/`](../shortcuts/review/README.md), the PR review workflow this pairs with.
- [`scripts/README.md`](../README.md) for the rest of the tooling.
- [`CONTRIBUTING.md`](../../CONTRIBUTING.md) for the PR process.
