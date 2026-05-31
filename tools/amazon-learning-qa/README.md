# Amazon Learning Q&A Local Product

This directory contains the repository-owned source for the local Amazon learning Q&A entry.
It intentionally does not include the local knowledge database, imported author articles,
run logs, screenshots, backups, or user notes.

## Local Data Boundary

By default the scripts look for the local knowledge workspace next to the repository:

```text
/Users/yangyingjia/OpenHuman/openhuman
/Users/yangyingjia/OpenHuman/openhuman-kb
```

You can override paths when running from another machine:

```bash
OPENHUMAN_KB_DIR=/path/to/openhuman-kb \
OPENHUMAN_REPO_DIR=/path/to/openhuman \
node tools/amazon-learning-qa/amazon-qa-product.mjs doctor
```

## Commands

Run these from the repository root:

```bash
pnpm amazon:doctor
pnpm amazon:start
pnpm amazon:smoke
pnpm amazon:test
pnpm amazon:handoff
```

The browser entry remains local:

```text
http://127.0.0.1:7790
```

## Deployment Boundary

This product depends on local SQLite files, Ollama, OpenHuman core binaries, and local source
documents. It is suitable for a local machine or VPS with those dependencies installed.
It is not a static Vercel deployment until the database, model runtime, and OpenHuman core
service are moved behind cloud services.
