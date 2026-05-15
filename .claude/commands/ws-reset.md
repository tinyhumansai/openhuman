---
description: Reset the current branch to main, fetch upstream, and update submodules.
allowed-tools: Bash
---

Run the following steps in order. Stop and surface the error if any step fails — do not paper over a failure.

```bash
bash scripts/shortcuts/ws-reset.sh
git status
```

The helper performs the destructive steps (`git fetch upstream`, `git checkout main`, `git reset --hard upstream/main`, `git submodule update --init --recursive`) behind a working-tree-dirty guard. Pass `--force` only if you intend to discard local changes.

Then report in one line: the new HEAD SHA of `main` and whether submodules changed.
