# Beginner's Guide to Contributing to OpenHuman

New to open source or coding? This guide walks you through everything from zero to your first pull request — based on real setup pain points that new contributors hit.

For the full contributor reference, see [`CONTRIBUTING.md`](CONTRIBUTING.md).

---

## Table of Contents

- [What is this project?](#what-is-this-project)
- [Step 1 — Install the required tools](#step-1--install-the-required-tools)
- [Step 2 — Fork and clone the repo](#step-2--fork-and-clone-the-repo)
- [Step 3 — Finish the local setup](#step-3--finish-the-local-setup)
- [Step 4 — Find an issue to work on](#step-4--find-an-issue-to-work-on)
- [Step 5 — Create your branch](#step-5--create-your-branch)
- [Step 6 — Make your change and verify it](#step-6--make-your-change-and-verify-it)
- [Step 7 — Push and open a Pull Request](#step-7--push-and-open-a-pull-request)
- [Keeping your fork up to date](#keeping-your-fork-up-to-date)
- [Troubleshooting common issues](#troubleshooting-common-issues)

---

## What is this project?

OpenHuman is a desktop AI assistant app. The codebase has three main parts:

| Part | Tech | What it does |
|------|------|-------------|
| `app/` | React + TypeScript | The UI — what you see and click |
| `app/src-tauri/` | Rust + Tauri | Wraps the UI into a desktop app |
| `src/` | Rust | The backend brain — logic, memory, RPC |

**As a beginner**, focus on `app/src/` (React/TypeScript). You don't need to touch Rust to make meaningful contributions.

---

## Step 1 — Install the required tools

### macOS (recommended for beginners)

Install [Homebrew](https://brew.sh) first if you don't have it:

```bash
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"
```

Then install everything the project needs:

```bash
# Node.js 24+ and pnpm (JavaScript package manager)
brew install node@24
npm install -g pnpm@10.10.0

# Rust (the backend language)
brew install rustup-init
rustup toolchain install 1.93.0 --profile minimal
rustup component add rustfmt clippy --toolchain 1.93.0

# CMake (required by Rust dependencies)
brew install cmake

# Xcode Command Line Tools (macOS build tools)
xcode-select --install
```

### Verify everything is installed

```bash
node --version     # should be v24.x.x or higher
pnpm --version     # should be 10.10.0
rustc --version    # should be 1.93.0
cmake --version    # any recent version
```

> **Node version warning**: The project requires Node 24+. If you see a warning like `Unsupported engine: wanted >=24.0.0 (current: v22.x.x)`, upgrade Node. Using `nvm`? Run `nvm install 24 && nvm use 24`.

---

## Step 2 — Fork and clone the repo

### 2a. Fork on GitHub

1. Go to [github.com/tinyhumansai/openhuman](https://github.com/tinyhumansai/openhuman)
2. Click **Fork** (top right)
3. This creates your own copy at `github.com/YOUR_USERNAME/openhuman`

### 2b. Clone your fork locally

```bash
git clone https://github.com/YOUR_USERNAME/openhuman.git
cd openhuman
```

### 2c. Add the upstream remote

This links your local copy to the original repo so you can pull in updates later:

```bash
git remote add upstream https://github.com/tinyhumansai/openhuman.git
```

Verify both remotes exist:

```bash
git remote -v
# origin    https://github.com/YOUR_USERNAME/openhuman.git  ← your fork
# upstream  https://github.com/tinyhumansai/openhuman.git   ← the original
```

---

## Step 3 — Finish the local setup

### 3a. Initialize submodules

The project includes vendored Tauri and CEF code as git submodules. You must do this before installing dependencies or desktop builds will fail:

```bash
git submodule update --init --recursive
```

### 3b. Install dependencies

```bash
pnpm install
```

> **pnpm not found?** If your shell can't find `pnpm` after installing it, run `export PATH="$PATH:$(npm root -g)/.bin"` and try again. Add that line to your `~/.zshrc` or `~/.bashrc` to make it permanent.

### 3c. Set up environment files

```bash
cp .env.example .env
cp app/.env.example app/.env.local
```

The defaults work for web-only development. You don't need to change anything to get started.

### 3d. Start the app

```bash
# Web UI only (easiest — runs in your browser)
pnpm dev

# Full desktop app (needs Rust + Tauri built first)
pnpm --filter openhuman-app dev:app
```

For your first contribution, `pnpm dev` is all you need.

---

## Step 4 — Find an issue to work on

1. Go to [github.com/tinyhumansai/openhuman/issues](https://github.com/tinyhumansai/openhuman/issues)
2. Filter by label — look for `good first issue`, `documentation`, or frontend-related labels
3. Read the issue fully before starting
4. Leave a comment saying you'd like to work on it — this avoids two people solving the same issue

### Recommended first areas for beginners

| Area | Where it lives | Skills needed |
|------|---------------|---------------|
| UI components | `app/src/` | React, TypeScript |
| Styles / design | `app/src/`, `app/tailwind.config.js` | CSS, Tailwind |
| Documentation | `*.md` files, `gitbooks/` | Writing |
| Bug fixes (frontend) | `app/src/` | React, TypeScript |

**Avoid for now**: anything in `src/` (Rust core) or `app/src-tauri/` (Tauri shell) until you're comfortable with the codebase.

---

## Step 5 — Create your branch

Always create a new branch for each issue. Never work directly on `main`.

```bash
# Make sure your main is up to date first
git fetch upstream
git checkout main
git pull --ff-only upstream main

# Create a branch named after what you're doing
git checkout -b fix/your-issue-description
# or
git checkout -b docs/your-doc-change
# or
git checkout -b feat/your-feature-name
```

---

## Step 6 — Make your change and verify it

After making your changes, run the relevant checks:

```bash
# For any change — check formatting
pnpm format:check

# For frontend code changes
pnpm typecheck   # TypeScript errors
pnpm lint        # Code style issues

# Auto-fix formatting issues
pnpm format
```

If you only changed documentation (`.md` files), `pnpm format:check` is the only check you need to run.

---

## Step 7 — Push and open a Pull Request

### Push your branch

```bash
git add .
git commit -m "your short description of what you changed"
git push -u origin your-branch-name
```

### Open the PR

1. Go to your fork on GitHub: `github.com/YOUR_USERNAME/openhuman`
2. You'll see a **"Compare & pull request"** banner — click it
3. Make sure the PR targets **`tinyhumansai/openhuman:main`** (not your fork)
4. Fill in the PR template completely
5. Link the issue with `Closes #ISSUE_NUMBER` in the description
6. Submit

---

## Keeping your fork up to date

Before starting any new work, sync your fork with the latest upstream changes:

```bash
git fetch upstream
git checkout main
git pull --ff-only upstream main
git push origin main
```

---

## Troubleshooting common issues

### `pnpm: command not found`

pnpm was installed but your shell can't find it. Fix:

```bash
export PATH="$PATH:$(npm root -g)/.bin"
```

Make it permanent by adding that line to `~/.zshrc`.

### `submodule` errors or missing Tauri vendor files

You skipped the submodule step. Run:

```bash
git submodule update --init --recursive
```

### Node version warning during `pnpm install`

The project requires Node 24+. Upgrade:

```bash
# If using nvm
nvm install 24
nvm use 24

# If using Homebrew
brew install node@24
```

### `pnpm typecheck` or `pnpm lint` errors on unchanged files

These may be pre-existing issues on `main` unrelated to your change. Note them in your PR description and proceed.

### Desktop build fails (`pnpm --filter openhuman-app dev:app`)

The desktop build requires the full Rust toolchain and vendored Tauri setup. For your first contributions, stick to `pnpm dev` (web mode) and skip the desktop build entirely.

---

## Still stuck?

- Join the [Discord](https://discord.tinyhumans.ai/) and ask in the contributors channel
- Comment on the issue you're working on
- Check [`gitbooks/developing/getting-set-up.md`](gitbooks/developing/getting-set-up.md) for deeper setup docs

Thank you for contributing to OpenHuman!
