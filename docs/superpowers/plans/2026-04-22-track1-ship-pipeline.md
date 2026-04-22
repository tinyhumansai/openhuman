# Track 1 — Ship Pipeline Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Unblock the OpenHuman ship pipeline — fix the failing Ubuntu installer smoke test, land four in-flight PRs in the right order, and wire Tauri auto-updater with signed Mac/Windows builds.

**Architecture:** Three independent workstreams that share a goal — getting code to users continuously. Workstream A diagnoses and fixes `scripts/install.sh`. Workstream B is sequenced PR landings. Workstream C wires the Tauri updater + signing into the existing GitHub Actions release workflow. A unblocks B; A and C can proceed in parallel.

**Tech Stack:** Bash + Python (install.sh), GitHub Actions, Tauri 2 updater plugin, Apple Developer ID + Windows code-signing certificate, `gh` CLI for PR ops.

---

## Workstream A — Fix Ubuntu installer smoke test

**Context.** Every PR currently fails the `Smoke install.sh (ubuntu-22.04)` job in CI. macOS and Windows install jobs pass; only Ubuntu fails. The script `scripts/install.sh` shells out to a Python3 parser to extract the right asset URL from `latest.json` (the GitHub release's metadata file). Suspected causes (in order of likelihood):

1. The asset for `x86_64-unknown-linux-gnu` is named differently than the parser expects.
2. The asset isn't fully propagated to the GitHub CDN at the moment of fetch.
3. The Python parser's JSON extraction is brittle to additions in `latest.json`.

The fix needs to produce a deterministic failure with the resolved URL when something goes wrong, and add a bounded retry on the asset HEAD probe.

### Task A1: Reproduce the failure locally

**Files:**
- Read: `scripts/install.sh`
- Read: `.github/workflows/installer-smoke.yml` (or wherever the smoke job lives)
- Read: a recent failed run's logs

- [ ] **Step 1: Find the smoke workflow file**

```bash
rg -l "install.sh" .github/workflows/
```
Expected: prints the workflow file(s) that invoke the installer.

- [ ] **Step 2: Pull the latest failed run logs for the smoke job**

```bash
gh run list --workflow="<workflow-file>.yml" --limit 5
gh run view <run-id> --log-failed | head -200
```
Expected: stderr/stdout of the failed Ubuntu step. Look for: the resolved download URL, the actual error (404? parse error? checksum mismatch?), and what `latest.json` looked like at that moment.

- [ ] **Step 3: Reproduce locally in a Linux container**

```bash
docker run --rm -it -v "$PWD":/repo -w /repo ubuntu:22.04 bash -c '
  apt-get update -qq && apt-get install -y -qq curl python3 ca-certificates jq &&
  bash scripts/install.sh
'
```
Expected: same failure as CI. Capture the stderr.

- [ ] **Step 4: Inspect what's actually in `latest.json` right now**

```bash
curl -fsSL https://github.com/tinyhumansai/openhuman/releases/latest/download/latest.json | jq .
```
Expected: prints the real JSON. Compare the asset names against what the script's Python parser greps for. Note the exact filename used for x86_64 Linux (e.g. `openhuman_<ver>_amd64.AppImage` vs `openhuman-x86_64-unknown-linux-gnu.tar.gz`).

- [ ] **Step 5: Document the root cause in your scratch notes**

Write one or two sentences in your local notes file (`/tmp/installer-rca.md`):
- What asset name the script expected
- What asset name actually exists
- Whether it was a name mismatch, a missing asset, or a propagation race

This locks in the diagnosis before you start changing code.

### Task A2: Add a failing test for the install resolver

**Files:**
- Create: `scripts/test_install.sh`

- [ ] **Step 1: Write the failing test**

```bash
#!/usr/bin/env bash
# scripts/test_install.sh — smoke-tests the install.sh resolver in isolation.
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# Use a fixture latest.json that mirrors what the real release publishes.
FIXTURE="$REPO_ROOT/scripts/fixtures/latest.json"
mkdir -p "$(dirname "$FIXTURE")"
cat > "$FIXTURE" <<'JSON'
{
  "version": "0.0.0-test",
  "platforms": {
    "linux-x86_64": {
      "url": "https://example.invalid/openhuman_0.0.0-test_amd64.AppImage",
      "signature": ""
    },
    "darwin-aarch64": {
      "url": "https://example.invalid/openhuman_0.0.0-test_aarch64.dmg",
      "signature": ""
    }
  }
}
JSON

# The resolver function should be sourced, not invoked end-to-end (no curl).
if ! source "$REPO_ROOT/scripts/install.sh" --source-only 2>/dev/null; then
  echo "FAIL: scripts/install.sh does not support --source-only mode"
  exit 1
fi

resolved=$(resolve_asset_url "$FIXTURE" "linux" "x86_64")
expected="https://example.invalid/openhuman_0.0.0-test_amd64.AppImage"
if [[ "$resolved" != "$expected" ]]; then
  echo "FAIL: expected $expected, got $resolved"
  exit 1
fi
echo "PASS"
```

- [ ] **Step 2: Run the test to verify it fails**

```bash
chmod +x scripts/test_install.sh
bash scripts/test_install.sh
```
Expected: FAIL — either `install.sh` doesn't support `--source-only`, or `resolve_asset_url` doesn't exist. This is the goal — we're adding both.

### Task A3: Refactor `install.sh` to expose a resolver and add retry-with-backoff

**Files:**
- Modify: `scripts/install.sh`

- [ ] **Step 1: Add the source-only guard at the very top of `scripts/install.sh`**

```bash
# scripts/install.sh — at the top, after the shebang and `set -euo pipefail`

# Allow tests to source this file without executing the install flow.
SOURCE_ONLY=0
for arg in "$@"; do
  if [[ "$arg" == "--source-only" ]]; then
    SOURCE_ONLY=1
  fi
done
```

- [ ] **Step 2: Extract the URL resolution into a function**

Find the existing inline Python that parses `latest.json` and replace it with a function:

```bash
# Resolves an asset URL from a latest.json file for a given OS/arch.
# Args: $1 = path to latest.json, $2 = os (linux|darwin|windows), $3 = arch (x86_64|aarch64)
# Stdout: the URL on success.
# Exit code: 0 on success; 2 on parse error (with diagnostic on stderr); 3 on missing platform.
resolve_asset_url() {
  local json_path="$1" os="$2" arch="$3"
  local key="${os}-${arch}"
  local url
  url=$(python3 - "$json_path" "$key" <<'PY'
import json, sys
path, key = sys.argv[1], sys.argv[2]
try:
    with open(path) as f:
        data = json.load(f)
except Exception as e:
    print(f"ERR_PARSE: {e}", file=sys.stderr)
    sys.exit(2)
plat = data.get("platforms", {}).get(key)
if not plat:
    available = ", ".join(sorted(data.get("platforms", {}).keys()))
    print(f"ERR_PLATFORM: {key} not in [{available}]", file=sys.stderr)
    sys.exit(3)
url = plat.get("url")
if not url:
    print(f"ERR_URL: no url field for {key}", file=sys.stderr)
    sys.exit(2)
print(url)
PY
  )
  local rc=$?
  if [[ $rc -ne 0 ]]; then
    return $rc
  fi
  printf '%s\n' "$url"
}

# Retries an HTTP HEAD on the asset URL, fails loudly with the URL.
verify_asset_reachable() {
  local url="$1" max_attempts=5 delay=2
  for i in $(seq 1 $max_attempts); do
    if curl -fsSI --max-time 10 "$url" >/dev/null 2>&1; then
      return 0
    fi
    if [[ $i -lt $max_attempts ]]; then
      sleep "$delay"
      delay=$((delay * 2))
    fi
  done
  echo "ERR_UNREACHABLE: $url not reachable after $max_attempts attempts" >&2
  return 4
}
```

- [ ] **Step 3: Wire the new functions into the install flow and skip if `SOURCE_ONLY=1`**

At the bottom of `install.sh`, gate the install execution:

```bash
if [[ "$SOURCE_ONLY" == "1" ]]; then
  return 0 2>/dev/null || exit 0
fi

# Existing install flow — replace the inline Python parse with calls to
# resolve_asset_url and verify_asset_reachable. Example shape:
LATEST_JSON=$(mktemp)
trap 'rm -f "$LATEST_JSON"' EXIT
curl -fsSL "https://github.com/tinyhumansai/openhuman/releases/latest/download/latest.json" -o "$LATEST_JSON"

OS=$(detect_os)         # existing helper — keep
ARCH=$(detect_arch)     # existing helper — keep
ASSET_URL=$(resolve_asset_url "$LATEST_JSON" "$OS" "$ARCH") || {
  echo "Failed to resolve asset URL for $OS-$ARCH" >&2
  cat "$LATEST_JSON" >&2
  exit 1
}
verify_asset_reachable "$ASSET_URL" || exit 1

echo "Installing from: $ASSET_URL"
# ... rest of existing download + install logic, using $ASSET_URL ...
```

(If `detect_os` / `detect_arch` aren't already named that, use the existing names. Don't rename — keep the diff minimal.)

- [ ] **Step 4: Run the test to verify it passes**

```bash
bash scripts/test_install.sh
```
Expected: PASS

- [ ] **Step 5: Run the install script end-to-end in a clean Ubuntu container**

```bash
docker run --rm -it -v "$PWD":/repo -w /repo ubuntu:22.04 bash -c '
  apt-get update -qq && apt-get install -y -qq curl python3 ca-certificates &&
  bash scripts/install.sh
'
```
Expected: either successful install, or a loud, specific error message naming the resolved URL. No silent Python tracebacks.

- [ ] **Step 6: Commit**

```bash
git add scripts/install.sh scripts/test_install.sh scripts/fixtures/latest.json
git commit -m "fix(install): resolver function + reachability retry + smoke test

Refactors scripts/install.sh to expose resolve_asset_url and
verify_asset_reachable. Adds scripts/test_install.sh that exercises
the resolver against a committed fixture latest.json. Failures now
report the resolved URL and the parse error instead of dying silently."
```

### Task A4: Wire the new test into CI

**Files:**
- Modify: the smoke workflow file from Task A1 Step 1

- [ ] **Step 1: Add a lint step that runs `test_install.sh` before the full smoke test**

In the workflow YAML, before the existing smoke step, add:

```yaml
      - name: Resolver unit test
        run: bash scripts/test_install.sh
```

- [ ] **Step 2: Push and verify CI is green on a draft PR**

```bash
git checkout -b fix/installer-smoke
git push -u origin fix/installer-smoke
gh pr create --draft --title "fix(install): unblock Ubuntu smoke test" \
  --body "Refactors install.sh, adds a unit test, fixes the resolver to fail loudly with the resolved URL on Ubuntu."
gh pr checks --watch
```
Expected: all checks green, including the new resolver unit test step.

- [ ] **Step 3: Mark ready, request review, merge once approved**

```bash
gh pr ready
gh pr merge --squash --delete-branch
```

---

## Workstream B — Land in-flight PRs

**Context.** Four PRs are waiting on Workstream A, listed here in the merge order from session triage.

### Task B1: Land #806 (CLAUDE.md slim)

- [ ] **Step 1: Verify CI is green on #806 after Workstream A lands**

```bash
gh pr checks 806
```
Expected: all green. If not, rebase onto main first:
```bash
git fetch origin && git checkout chore/claude-md-slim && git rebase origin/main && git push --force-with-lease
```

- [ ] **Step 2: Merge**

```bash
gh pr merge 806 --squash --delete-branch
```

### Task B2: Land #786 (RPC test hardening)

- [ ] **Step 1: Rebase onto the new main, verify CI**

```bash
gh pr checkout 786
git fetch origin && git rebase origin/main && git push --force-with-lease
gh pr checks --watch
```
Expected: all green.

- [ ] **Step 2: Merge**

```bash
gh pr merge 786 --squash --delete-branch
```

### Task B3: Retrigger #788 (home next-steps)

- [ ] **Step 1: Rebase, push empty commit if needed to retrigger CI**

```bash
gh pr checkout 788
git fetch origin && git rebase origin/main
git commit --allow-empty -m "ci: retrigger after installer fix"
git push --force-with-lease
gh pr checks --watch
```
Expected: green.

- [ ] **Step 2: Merge**

```bash
gh pr merge 788 --squash --delete-branch
```

### Task B4: Debug and land #797 (threads schema parse contract tests)

**Context.** This PR has a real test failure (`resolve_dirs_uses_active_user_when_present`) that the installer fix won't resolve. The failure indicates the test's expectation of which config dir is "active" doesn't match what the runtime resolves on the CI box.

- [ ] **Step 1: Reproduce the test failure locally**

```bash
gh pr checkout 797
git fetch origin && git rebase origin/main
cargo test --manifest-path Cargo.toml resolve_dirs_uses_active_user_when_present -- --nocapture
```
Expected: test fails. Read the diff between the expected and actual paths.

- [ ] **Step 2: Read the resolver implementation**

```bash
rg -n "fn resolve_dirs" src/
rg -n "active_user" src/openhuman/config/
```
Expected: locates the function under test and the "active user" concept it consults.

- [ ] **Step 3: Identify whether the test or the resolver is wrong**

Inspect the test setup. Likely scenario: the test sets up an active user via env var or config file, but the resolver short-circuits to `$XDG_CONFIG_HOME` or similar before consulting the active user. Either:
- The test isn't priming the right state — fix the test setup.
- The resolver has a bug where it doesn't honor the active user — fix the resolver.

Pick one based on what the spec/intent of `resolve_dirs` is (read its doc comment / surrounding code). Document your decision in the PR's description.

- [ ] **Step 4: Apply the minimal fix and verify**

Make the change, run the failing test:
```bash
cargo test --manifest-path Cargo.toml resolve_dirs_uses_active_user_when_present -- --nocapture
```
Expected: PASS. Then run the full test module to make sure you didn't regress neighbors:
```bash
cargo test --manifest-path Cargo.toml -- config::
```

- [ ] **Step 5: Push and merge**

```bash
git add -u && git commit -m "fix(config): honor active_user in resolve_dirs"
git push
gh pr checks --watch
gh pr merge 797 --squash --delete-branch
```

---

## Workstream C — Tauri auto-updater + signed builds

**Context.** `app/src-tauri/tauri.conf.json` has `"updater": { "active": false }`. macOS and Windows release builds are unsigned (or signed only with ad-hoc certs), which means the updater couldn't verify them anyway. This workstream wires the Tauri updater plugin end-to-end.

### Task C1: Generate the updater signing keypair

**Files:** none in repo; key material goes into 1Password / GitHub Secrets.

- [ ] **Step 1: Generate the keypair**

```bash
yarn tauri signer generate -w ~/.tauri/openhuman-updater.key
```
Expected: prints the public key to stdout, writes the private key to the path. **Set a strong password when prompted.**

- [ ] **Step 2: Store the private key and password in GitHub Secrets**

```bash
gh secret set TAURI_PRIVATE_KEY < ~/.tauri/openhuman-updater.key
gh secret set TAURI_KEY_PASSWORD --body "<the password from step 1>"
```
Expected: secrets set on the repo.

- [ ] **Step 3: Save the public key to a known location for embedding**

Copy the public key string (printed in step 1) — it goes into `tauri.conf.json` in Task C2.

### Task C2: Enable updater in `tauri.conf.json`

**Files:**
- Modify: `app/src-tauri/tauri.conf.json`

- [ ] **Step 1: Set updater config**

```json
{
  "plugins": {
    "updater": {
      "active": true,
      "endpoints": [
        "https://github.com/tinyhumansai/openhuman/releases/latest/download/latest.json"
      ],
      "dialog": true,
      "pubkey": "<paste the public key string from Task C1 Step 1 here>"
    }
  }
}
```

(If the existing `tauri.conf.json` has `"updater": { "active": false }` at top level rather than under `plugins`, use the schema location it already uses — Tauri 2 supports both shapes depending on plugin version. Match the existing structure.)

- [ ] **Step 2: Add the updater plugin dependency in `app/src-tauri/Cargo.toml`**

```toml
[dependencies]
tauri-plugin-updater = "2"
```

- [ ] **Step 3: Initialize the plugin in `app/src-tauri/src/lib.rs`**

In the `tauri::Builder::default()` chain, add:

```rust
.plugin(tauri_plugin_updater::Builder::new().build())
```

- [ ] **Step 4: Verify `cargo check`**

```bash
cargo check --manifest-path app/src-tauri/Cargo.toml
```
Expected: compiles clean.

- [ ] **Step 5: Commit**

```bash
git add app/src-tauri/tauri.conf.json app/src-tauri/Cargo.toml app/src-tauri/Cargo.lock app/src-tauri/src/lib.rs
git commit -m "feat(updater): enable Tauri updater with signed release endpoint"
```

### Task C3: Sign Mac builds in the release workflow

**Files:**
- Modify: `.github/workflows/release.yml` (or whichever workflow builds the Tauri release)

**Prereqs:** Apple Developer ID Application certificate exported as base64 into GitHub Secret `MACOS_CERTIFICATE`, password as `MACOS_CERTIFICATE_PWD`, signing identity (e.g. `Developer ID Application: Tinyhumans, Inc. (XXXXXX)`) as `APPLE_SIGNING_IDENTITY`, and an app-specific password for notarization as `APPLE_PASSWORD` with `APPLE_ID` and `APPLE_TEAM_ID`.

- [ ] **Step 1: Add cert import + signing env to the macos build job**

Inside the existing macos job, before the `tauri-action` (or `yarn tauri build`) step, add:

```yaml
      - name: Import Apple certificate
        env:
          MACOS_CERTIFICATE: ${{ secrets.MACOS_CERTIFICATE }}
          MACOS_CERTIFICATE_PWD: ${{ secrets.MACOS_CERTIFICATE_PWD }}
        run: |
          KEYCHAIN_PATH="$RUNNER_TEMP/build.keychain"
          KEYCHAIN_PWD=$(openssl rand -base64 32)
          echo "$MACOS_CERTIFICATE" | base64 --decode > "$RUNNER_TEMP/cert.p12"
          security create-keychain -p "$KEYCHAIN_PWD" "$KEYCHAIN_PATH"
          security set-keychain-settings -lut 21600 "$KEYCHAIN_PATH"
          security unlock-keychain -p "$KEYCHAIN_PWD" "$KEYCHAIN_PATH"
          security import "$RUNNER_TEMP/cert.p12" -P "$MACOS_CERTIFICATE_PWD" \
            -A -t cert -f pkcs12 -k "$KEYCHAIN_PATH"
          security set-key-partition-list -S apple-tool:,apple: -s -k "$KEYCHAIN_PWD" "$KEYCHAIN_PATH"
          security list-keychains -d user -s "$KEYCHAIN_PATH" $(security list-keychains -d user | tr -d '"')
```

And on the Tauri build step, set the env:

```yaml
      - name: Build Tauri (macos)
        env:
          APPLE_SIGNING_IDENTITY: ${{ secrets.APPLE_SIGNING_IDENTITY }}
          APPLE_ID: ${{ secrets.APPLE_ID }}
          APPLE_PASSWORD: ${{ secrets.APPLE_PASSWORD }}
          APPLE_TEAM_ID: ${{ secrets.APPLE_TEAM_ID }}
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_KEY_PASSWORD }}
        run: yarn tauri build --target universal-apple-darwin
```

- [ ] **Step 2: Push, run the workflow on a release tag, and verify**

```bash
git tag v0.0.0-test-signing
git push origin v0.0.0-test-signing
gh run watch
```
Expected: macOS DMG built, signed, notarized. Download from the resulting release and verify:

```bash
codesign --verify --deep --strict --verbose=2 /path/to/openhuman.app
spctl --assess --type execute --verbose /path/to/openhuman.app
```
Expected: both report "accepted".

- [ ] **Step 3: Delete the test tag**

```bash
git push --delete origin v0.0.0-test-signing
git tag -d v0.0.0-test-signing
gh release delete v0.0.0-test-signing --yes
```

### Task C4: Sign Windows builds in the release workflow

**Files:**
- Modify: `.github/workflows/release.yml` (windows job)

**Prereqs:** Windows code-signing certificate (EV or OV) exported as base64 in `WINDOWS_CERTIFICATE`, password as `WINDOWS_CERTIFICATE_PWD`.

- [ ] **Step 1: Add the Windows signing env to the windows build job**

```yaml
      - name: Build Tauri (windows)
        env:
          TAURI_SIGNING_PRIVATE_KEY: ${{ secrets.TAURI_PRIVATE_KEY }}
          TAURI_SIGNING_PRIVATE_KEY_PASSWORD: ${{ secrets.TAURI_KEY_PASSWORD }}
        run: yarn tauri build
```

And in `app/src-tauri/tauri.conf.json` under the Windows bundle config:

```json
{
  "bundle": {
    "windows": {
      "certificateThumbprint": "<thumbprint of the cert>",
      "digestAlgorithm": "sha256",
      "timestampUrl": "http://timestamp.digicert.com"
    }
  }
}
```

(For SignTool-based signing — adjust per your CA's instructions if using azuresigntool / KSP.)

- [ ] **Step 2: Test-tag, verify signed binary**

```bash
git tag v0.0.0-test-signing-win
git push origin v0.0.0-test-signing-win
gh run watch
```
Download the produced `.msi` or `.exe`, then on a Windows machine:
```powershell
Get-AuthenticodeSignature .\OpenHuman_*.msi
```
Expected: `Status: Valid`, signer matches your cert.

- [ ] **Step 3: Clean up the test tag** (same as C3 step 3)

### Task C5: End-to-end updater test

**Files:** none in repo — manual verification.

- [ ] **Step 1: Cut a real release at version N**

```bash
git checkout main && git pull
yarn version --new-version 0.X.0   # bump
git push --follow-tags
gh run watch  # release workflow runs
```
Expected: GitHub release created with signed Mac/Windows artifacts and a `latest.json` pointing at them.

- [ ] **Step 2: Install version N on a clean Mac**

Download the DMG from the release, install. Launch — confirm version is N.

- [ ] **Step 3: Cut version N+1**

```bash
yarn version --new-version 0.Y.0   # bump
git push --follow-tags
gh run watch
```
Expected: release N+1 created with updated `latest.json`.

- [ ] **Step 4: Launch the installed N app and verify update prompt**

Within ~60s of launch (or on next launch), the Tauri updater dialog should appear: "A new version is available." Accept it. App restarts. Confirm version is now N+1.

- [ ] **Step 5: Repeat steps 2-4 on Windows**

Same flow on a clean Windows VM.

- [ ] **Step 6: Document the verified flow**

Add a short note to `docs/RELEASE_POLICY.md`:

```markdown
## Updater verification (2026-04-22)

- Mac: codesign + notarized + auto-update verified end-to-end on macOS 15.x.
- Windows: Authenticode signed + auto-update verified end-to-end on Windows 11.
- Updater pubkey: <fingerprint>; private key in 1Password vault `openhuman-release`.
```

- [ ] **Step 7: Commit**

```bash
git add docs/RELEASE_POLICY.md
git commit -m "docs(release): record updater end-to-end verification on mac and windows"
```

---

## Acceptance for Track 1

- [ ] Ubuntu installer smoke test green on 3 consecutive PR pushes
- [ ] PRs #806, #786, #788, #797 all merged into main
- [ ] An installed v(N) build auto-prompts and applies v(N+1) end-to-end on Mac and Windows
- [ ] `RELEASE_POLICY.md` updated with verification record
