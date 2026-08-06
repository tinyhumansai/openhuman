# AppImage sharun library-path repair implementation plan

> Approved design:
> `docs/specs/2026-07-24-appimage-sharun-lib-path-design.md` at
> `baae2b02c73e48826e156a111b7e8aff82f214b9`.

## Goal

Repair the Linux AppImage postprocessor so sharun's generated
`shared/lib/lib.path` contains only AppDir-anchored `+` entries, reject malformed
paths before release, and prove that the final repacked amd64 AppImage reaches
normal application startup when invoked from outside its extracted AppDir.

The implementation changes only the AppImage release path:

- `scripts/release/strip-appimage-graphics-libs.sh`
- `scripts/release/test-strip-appimage-rpaths.sh`
- a new final-artifact validator at
  `scripts/release/validate-appimage-runtime.sh`
- the Linux portion of `.github/workflows/build-desktop.yml`

The existing graphics-library removal, interpreter replacement, RPATH repair,
artifact signing, updater tarball rebuilding, x86_64/aarch64 build matrix, and
non-Linux workflow branches remain in place.

## Scope guardrails

This plan intentionally does **not** implement the broader generated triage plan
currently embedded in issue #5037:

- The unbundled `openhuman-core`/AUR failure remains a downstream packaging and
  optional-`enigo` follow-up. Do not change `Cargo.toml`, feature gates, voice
  input, computer tools, `INSTALL.md`, or AUR metadata in this work.
- Do not change bundled `libssl.so.3`/`libcrypto.so.3`, NSS exclusions, or claim
  to fix the later OpenSSL symbol-skew/segfault behavior in #3716. The smoke
  catches an immediate loader failure, not every later CEF/OpenSSL crash.
- Do not pin the mutable `quick-sharun.sh` or uruntime downloads in the vendored
  Tauri bundler. Supply-chain pinning is a separate follow-up.
- Do not add an Arch/Fedora/Tumbleweed matrix or make
  `.github/workflows/pr-quality.yml`'s advisory AppImage guard blocking. The
  release-builder smoke is the selected acceptance test.
- Do not replace sharun with a custom `LD_LIBRARY_PATH` wrapper.

## Confirmed sharun contract

Pin the tests and implementation to the actual sharun grammar:

- `shared/lib/lib.path` is one entry per line.
- `+` represents the directory containing `lib.path`, which is the extracted
  AppDir's canonical `shared/lib` directory.
- `+/subdir` represents a descendant of that directory.
- sharun trims the complete file, replaces newlines with `:`, and then replaces
  every `+` with the absolute `shared/lib` path.
- `+` is therefore a marker inside an entry, **not** an entry delimiter.

The current `tr '+:' '\n\n'` parser destroys this marker and must be removed.
Tests must use the accepted textual forms `+` and `+/subdir`, with newline
separation. They must not introduce an invented plus- or colon-separated input
format.

## Sequencing decision

`strip-appimage-graphics-libs.sh` currently performs four phases in one command:

1. extract and postprocess an AppDir;
2. repack and replace the final `.AppImage`;
3. re-sign the `.AppImage`;
4. rebuild and re-sign its updater tarball.

The final-artifact validator must run after phase 2 but before phase 3. Add the
validator call immediately after the rebuilt file is moved over the original
and before the image is appended to `MODIFIED_PATHS`. This satisfies both
requirements that the inspected bytes are the uploaded bytes and that a bad
launcher fails before signing. It also avoids a defer/finalize mode that could
accidentally validate one image and then repack another.

Static validation runs for every Linux AppImage. The executable smoke is
enabled only for `x86_64-unknown-linux-gnu` in this issue because that is the
reported architecture and acceptance target. The helper remains
architecture-neutral: a later change can enable the same smoke on arm64 by
setting the same environment flag on an executable arm64 runner.

---

## Task 1: Replace the destructive parser with sharun-native normalization

**Files:**

- Modify: `scripts/release/test-strip-appimage-rpaths.sh`
- Modify: `scripts/release/strip-appimage-graphics-libs.sh`

### Step 1.1: Add failing normalization fixtures

Extend `test-strip-appimage-rpaths.sh` after the host-ELF fixture is established
and the target script is sourced. Add a reusable released-style AppDir builder:

```bash
make_sharun_appdir() {
  local appdir="$1"
  mkdir -p "$appdir/shared/bin" "$appdir/shared/lib/plugins" "$appdir/bin"

  cp "$HOST_ELF" "$appdir/sharun"
  printf 'Interpreter not found!' >> "$appdir/sharun"
  chmod +x "$appdir/sharun"
  ln "$appdir/sharun" "$appdir/AppRun"
  ln "$appdir/sharun" "$appdir/bin/OpenHuman"

  cp "$HOST_ELF" "$appdir/shared/bin/OpenHuman"
  chmod +x "$appdir/shared/bin/OpenHuman"
}
```

Use separate fixture directories so one failure cannot contaminate the next.
Add these assertions:

1. An input with one entry
   `"$WORK/squashfs-root/shared/lib"` rewrites to exactly `+`.
2. An input rooted in the bundler staging layout
   `"$WORK/appimage_deb/data/usr/lib"` rewrites to exactly `+`.
3. AppDir descendants map as follows:
   - `"$WORK/squashfs-root/shared/lib/plugins"` becomes `+/plugins`;
   - `"$WORK/appimage_deb/data/usr/lib/plugins"` becomes `+/plugins`.
4. A file containing these newline-delimited entries:

   ```text
   +
   +/plugins
   +
   /opt/unrelated/lib
   ```

   normalizes to exactly:

   ```text
   +
   +/plugins
   ```

   Stable first-seen order is preserved and the unrelated host path is
   excluded.

5. Running `rewrite_sharun_lib_path` a second time leaves the bytes unchanged
   and returns the no-change status expected by `strip_one_appimage`.
6. An input containing only `/opt/unrelated/lib` fails normalization and is not
   replaced with `shared/lib`.

Capture expected failures in subshells because the release helpers are allowed
to terminate fail-closed:

```bash
if ( rewrite_sharun_lib_path "$fixture" ) 2>"$WORK/rewrite.err"; then
  fail "rewrite accepted a lib.path with no AppDir library root"
fi
grep -F "no valid AppDir library entries" "$WORK/rewrite.err" >/dev/null \
  || fail "rewrite failure did not explain why lib.path was rejected"
```

Run the focused test on Linux:

```bash
bash scripts/release/test-strip-appimage-rpaths.sh
```

Expected RED: at least the canonical-marker case fails because the current
implementation splits on `+` and writes `shared/lib`; the no-valid-root case
also fails because the current fallback silently writes `shared/lib`.

On macOS, first run:

```bash
bash scripts/release/test-strip-appimage-rpaths.sh
```

Expected local result: a clean `SKIP` when `patchelf` or a host ELF is
unavailable. Do not treat that skip as execution evidence; the RED/GREEN result
for this task must come from the existing Ubuntu PR job or another Linux host.

### Step 1.2: Implement newline-based normalization

Refactor `rewrite_sharun_lib_path` without changing its public signature:

```bash
rewrite_sharun_lib_path() {
  local appdir="$1"
  # Return 1 for non-sharun/no-file/no-change.
  # Return 0 after writing a changed valid file.
  # Exit non-zero with an ERROR for malformed input from which no valid
  # AppDir-rooted entry can be derived.
}
```

Implement the following exact algorithm:

1. Keep the `uses_sharun_launcher` and non-empty-file guards.
2. Read with `while IFS= read -r entry || [ -n "$entry" ]`; do not use `tr`,
   word splitting, `eval`, or colon splitting.
3. Reject empty interior lines, carriage returns, embedded `:`, and surrounding
   whitespace as malformed rather than letting the loader interpret them
   differently from the validator.
4. Normalize each entry:
   - `+` stays `+`;
   - `+/suffix` stays `+/suffix` after the descendant checks in Task 2;
   - an absolute path containing `/squashfs-root/shared/lib` maps that root to
     `+` and its suffix to `+/suffix`;
   - an absolute path containing `/appimage_deb/data/usr/lib` or `/data/usr/lib`
     maps that root to `+` and its suffix to `+/suffix`;
   - another absolute path is dropped;
   - a bare relative path is malformed and causes normalization to fail.
5. For derived suffixes, remove one leading slash and no other content. A root
   with an empty suffix becomes `+`.
6. Deduplicate using exact whole-entry equality while preserving first-seen
   order. Use an indexed Bash array and a small equality loop so the script
   remains syntax-compatible with macOS Bash 3.2; do not introduce associative
   arrays.
7. If no valid AppDir-rooted entries remain, print:

   ```text
   [strip-libs] ERROR: shared/lib/lib.path contains no valid AppDir library entries
   ```

   and return non-zero without modifying the file.

8. Join entries with newline characters and write through a temporary file in
   the same directory followed by `mv`. Preserve the final format as one entry
   per line; a trailing newline is acceptable but must be deterministic.
9. Compare the normalized temporary file with the original using `cmp -s`.
   Delete the temporary and return `1` on an idempotent no-op. On change, move
   it into place, log the normalized entries, and return `0`.

Remove the early grep that only invokes rewriting when `/home/runner` or `/__w`
is present. Valid marker files still need the idempotency path, and malformed
bare-relative files must not bypass normalization.

### Step 1.3: Run the GREEN checks

Linux:

```bash
bash -n scripts/release/strip-appimage-graphics-libs.sh
bash -n scripts/release/test-strip-appimage-rpaths.sh
bash scripts/release/test-strip-appimage-rpaths.sh
```

Expected GREEN: all old RPATH/required-library tests and all new normalization
cases pass.

macOS:

```bash
bash -n scripts/release/strip-appimage-graphics-libs.sh
bash -n scripts/release/test-strip-appimage-rpaths.sh
bash scripts/release/test-strip-appimage-rpaths.sh
```

Expected: both syntax checks pass and the execution test skips cleanly if Linux
ELF tooling is absent.

### Step 1.4: Commit the normalization

```bash
atomic-commit "fix(appimage): emit sharun-native library paths" -- \
  scripts/release/test-strip-appimage-rpaths.sh \
  scripts/release/strip-appimage-graphics-libs.sh
```

---

## Task 2: Make `lib.path` validation fail closed and cover the shipped ELF launcher

**Files:**

- Modify: `scripts/release/test-strip-appimage-rpaths.sh`
- Modify: `scripts/release/strip-appimage-graphics-libs.sh`

### Step 2.1: Add failing validator and launcher-layout cases

Reuse `make_sharun_appdir` and add a table-driven helper:

```bash
assert_lib_path_rejected() {
  local label="$1"
  local value="$2"
  local expected_entry="$3"
  local fixture="$WORK/reject-$label"
  make_sharun_appdir "$fixture"
  printf '%s\n' "$value" >"$fixture/shared/lib/lib.path"

  if ( validate_sharun_lib_path "$fixture" ) 2>"$fixture/error.log"; then
    fail "validate_sharun_lib_path accepted $label"
  fi
  grep -F "$expected_entry" "$fixture/error.log" >/dev/null \
    || fail "$label error did not identify offending entry '$expected_entry'"
}
```

Add rejection cases for:

- `shared/lib` (bare relative);
- `/home/runner/work/openhuman/squashfs-root/shared/lib` (CI absolute);
- `/__w/openhuman/openhuman/appimage_deb/data/usr/lib` (CI absolute);
- `+/../outside` (parent traversal);
- `+/plugins/../../outside` (nested traversal);
- `+/missing` (marker-relative directory absent from the fixture);
- `+/plugins:shared/lib` (embedded loader separator);
- ` +` and `+ ` (whitespace changes loader semantics).

Each error assertion must match the complete offending entry in the diagnostic,
not only a generic error category.

Add accepted cases:

- `+`;
- `+` followed by `+/plugins` on a new line;
- a normalized file passed twice, proving validation is non-mutating.

Add explicit released-layout assertions:

```bash
[ "$fixture/AppRun" -ef "$fixture/sharun" ] \
  || fail "AppRun fixture is not hard-linked to sharun"
[ "$fixture/bin/OpenHuman" -ef "$fixture/sharun" ] \
  || fail "bin/OpenHuman fixture is not hard-linked to sharun"
is_executable_elf "$fixture/AppRun" \
  || fail "released-style AppRun fixture is not ELF"
uses_sharun_launcher "$fixture" \
  || fail "released-style hard-linked ELF launcher was not detected"
```

Write `+` to its `lib.path`, validate it, change to an unrelated directory, and
model sharun's pinned expansion:

```bash
caller="$WORK/caller"
mkdir -p "$caller"
(
  cd "$caller"
  expanded="$(
    sed "s|+|$fixture/shared/lib|g" "$fixture/shared/lib/lib.path" |
      paste -sd ':' -
  )"
  [ "$expanded" = "$fixture/shared/lib" ] \
    || fail "sharun marker did not expand beneath the fixture AppDir"
)
```

Also assert that `patch_apprun_sharun_cwd "$fixture"` returns its no-change
status and does not mutate the ELF inode. This pins the compatibility helper as
shell-wrapper-only hardening, not the fix for released ELF AppRuns.

Run on Linux:

```bash
bash scripts/release/test-strip-appimage-rpaths.sh
```

Expected RED: the current validator accepts bare relatives, traversal, missing
directories, and an ELF AppRun without any meaningful library-path checks.

### Step 2.2: Implement exact fail-closed validation

Keep the `validate_sharun_lib_path "$appdir"` interface. Parse the file by
newline using the same helper/loop as normalization so rewrite and validation
cannot disagree.

For every non-empty entry:

1. Accept only `+` or a string beginning `+/`.
2. Reject `:`, carriage returns, leading/trailing whitespace, absolute paths,
   and bare relative paths.
3. Split the marker-relative suffix on `/` and reject empty, `.`, or `..`
   components. This rejects traversal before filesystem canonicalization.
4. Resolve `+` to `$appdir/shared/lib`; resolve `+/suffix` to
   `$appdir/shared/lib/suffix`.
5. Require the resolved path to be a directory.
6. Canonicalize both the library root and resolved directory with `realpath`,
   then require the result to equal the root or start with
   `"$canonical_root/"`. This prevents an in-tree symlink from escaping the
   AppDir even when the textual path contains no `..`.
7. On failure, print one line with the offending entry:

   ```text
   [strip-libs] ERROR: invalid sharun lib.path entry '<entry>': <reason>
   ```

   and return/exit non-zero before repacking.

Require at least one entry and retain the existing missing-file diagnostic.
Do not mutate the file in validation.

Update the comment above `patch_apprun_sharun_cwd` and its success/warning logs:

- state that valid `+` paths are the primary released-layout fix;
- state that the helper remains for historical shell `AppRun` layouts;
- remove claims that changing CWD is required for an ELF sharun launcher;
- preserve the current shell patch behavior and return convention.

### Step 2.3: Run GREEN checks

Linux:

```bash
bash -n scripts/release/strip-appimage-graphics-libs.sh
bash scripts/release/test-strip-appimage-rpaths.sh
```

Expected GREEN: accepted marker paths pass; every malformed entry fails with
the exact entry in stderr; the ELF hard-link fixture is detected and remains
unchanged by the shell compatibility patch.

macOS:

```bash
bash -n scripts/release/strip-appimage-graphics-libs.sh
bash -n scripts/release/test-strip-appimage-rpaths.sh
bash scripts/release/test-strip-appimage-rpaths.sh
```

Expected: syntax passes and the execution test cleanly skips if required.

### Step 2.4: Commit fail-closed validation

```bash
atomic-commit "test(appimage): reject malformed sharun library paths" -- \
  scripts/release/test-strip-appimage-rpaths.sh \
  scripts/release/strip-appimage-graphics-libs.sh
```

---

## Task 3: Inspect and smoke-test the final repacked AppImage before signing

**Files:**

- Create: `scripts/release/validate-appimage-runtime.sh`
- Modify: `scripts/release/test-strip-appimage-rpaths.sh`
- Modify: `scripts/release/strip-appimage-graphics-libs.sh`

### Step 3.1: Add failing tests for extracted-layout validation

The new script must be sourceable for fixture tests and executable for a real
AppImage. Before implementing it, extend
`test-strip-appimage-rpaths.sh` to source it after sourcing the postprocessor:

```bash
RUNTIME_VALIDATOR="$SCRIPT_DIR/validate-appimage-runtime.sh"
[ -f "$RUNTIME_VALIDATOR" ] \
  || fail "$RUNTIME_VALIDATOR not found"
# shellcheck source=/dev/null
source "$RUNTIME_VALIDATOR"
```

Add fixture tests for the function interface:

```bash
validate_extracted_appdir "$appdir"
validate_final_appimage "$appimage"
smoke_extracted_apprun "$appdir" "$foreign_cwd" "$log_file"
```

`validate_extracted_appdir` is the unit-test seam; it must not extract or launch
anything. Build a complete fixture with:

- hard-linked ELF `AppRun`, `sharun`, and `bin/OpenHuman`;
- executable ELF `shared/bin/OpenHuman`;
- `shared/lib/lib.path` containing `+`;
- `shared/lib/anylinux.so`;
- `shared/lib/libxdo.so.3`;
- `shared/lib/libcef.so`;
- clean RPATHs on all copied ELFs.

Because `/bin/true` does not naturally declare the production `NEEDED`
libraries, make the needed-name check injectable only in fixture mode:

```bash
APPIMAGE_EXPECTED_NEEDED="${APPIMAGE_EXPECTED_NEEDED:-libxdo.so.3 libcef.so}"
```

Set `APPIMAGE_EXPECTED_NEEDED=""` only around unit fixtures. The executable CLI
and release workflow must leave the production default intact.

Assert the complete fixture passes. Then clone it into isolated cases and
assert rejection when:

- `anylinux.so` is absent;
- `libxdo.so.3` is absent;
- `libcef.so` is absent;
- `shared/bin/OpenHuman` is absent or is not ELF;
- `AppRun` is neither the same inode nor byte-equivalent to `sharun`;
- `bin/OpenHuman` is neither the same inode nor byte-equivalent to `sharun`;
- an ELF RPATH contains
  `/home/runner/work/openhuman/openhuman/shared/lib`;
- an ELF RPATH contains `/__w/openhuman/openhuman/shared/lib`;
- the real app's `NEEDED` list lacks `libxdo.so.3`;
- the real app's `NEEDED` list lacks `libcef.so`.

For equivalent launcher support, copy rather than hard-link `sharun` to
`AppRun` and `bin/OpenHuman`; byte-identical executable copies must pass. This
covers a repacker that preserves launcher contents but not hard-link metadata.

Add a source-order regression that invokes a stub validator from
`strip_one_appimage` and proves it runs after the final `mv` but before the path
is appended to `MODIFIED_PATHS`. Do this through an overridable command:

```bash
APPIMAGE_RUNTIME_VALIDATOR="${APPIMAGE_RUNTIME_VALIDATOR:-$SCRIPT_DIR/validate-appimage-runtime.sh}"
```

The test stub must record that the supplied final path exists and that
`MODIFIED_PATHS` is still empty. Do not execute or repack a real AppImage in
this unit test; factor the post-repack handoff into:

```bash
validate_rebuilt_appimage() {
  local rebuilt_path="$1"
  "$APPIMAGE_RUNTIME_VALIDATOR" "$rebuilt_path"
}
```

Call this seam directly with a fixture file to pin its command/argument
contract, and use a source-text order assertion for the three adjacent
statements:

```text
mv "$rebuilt" "$original"
validate_rebuilt_appimage "$original"
MODIFIED_PATHS+=("$original")
```

Run on Linux:

```bash
bash scripts/release/test-strip-appimage-rpaths.sh
```

Expected RED: the validator file/functions and the post-repack handoff do not
exist.

### Step 3.2: Implement the sourceable final-artifact validator

Create `scripts/release/validate-appimage-runtime.sh` with `set -euo pipefail`,
three shell functions, and a direct-execution guard. The exact interfaces are:

- `validate_extracted_appdir "$appdir"` performs the eight ordered static checks
  immediately below.
- `smoke_extracted_apprun "$appdir" "$foreign_cwd" "$log_file"` performs the
  eight ordered launch checks immediately below.
- `validate_final_appimage "$image"` performs the five ordered extraction
  checks immediately below.

Use this direct-execution guard:

```bash
if [ "${BASH_SOURCE[0]}" = "$0" ]; then
  [ "$#" -eq 1 ] || {
    echo "Usage: $0 <final.AppImage>" >&2
    exit 2
  }
  validate_final_appimage "$1"
fi
```

Do not duplicate `lib.path` grammar. Resolve the script directory and source
`strip-appimage-graphics-libs.sh`, whose direct-execution guard prevents
`main()` from running. Reuse:

- `is_elf`;
- `is_executable_elf`;
- `uses_sharun_launcher`;
- `validate_sharun_lib_path`;
- `validate_appimage_required_libs`.

Implement `validate_extracted_appdir "$appdir"` as follows:

1. Print the extracted path and the normalized `lib.path` contents with each
   line prefixed for readable CI logs.
2. Require `uses_sharun_launcher "$appdir"`.
3. Require ELF executables at `AppRun`, `sharun`, `bin/OpenHuman`, and
   `shared/bin/OpenHuman`.
4. For both launcher aliases, accept either `[ alias -ef sharun ]` or
   `cmp -s alias sharun`; reject anything else.
5. Call `validate_sharun_lib_path` and
   `validate_appimage_required_libs`.
6. Add `anylinux.so` to the required-library contract in
   `validate_appimage_required_libs`; its missing diagnostic must identify the
   preload library separately from the existing libxdo/CEF diagnostics.
7. Run `patchelf --print-needed "$appdir/shared/bin/OpenHuman"` and require each
   whitespace-delimited name in
   `${APPIMAGE_EXPECTED_NEEDED:-libxdo.so.3 libcef.so}` as an exact full line.
   Print the NEEDED list before checking it.
8. Walk ELF files under `shared/bin`, `shared/lib`, `bin`, `lib`, `usr/bin`, and
   `usr/lib`. For each, read RPATH with `patchelf --print-rpath`; reject
   `/home/runner/` and `/__w/`. This is validation only: never patch the final
   artifact.

`validate_final_appimage "$image"` must:

1. Resolve the input with `realpath`, require an executable regular file, and
   require `patchelf`.
2. Create one temporary root and a distinct `foreign-cwd` below it; install a
   trap that removes only that exact temporary root.
3. Change into the foreign CWD before invoking
   `"$image" --appimage-extract`, redirecting extraction output to a log. This
   ensures extraction itself does not rely on the artifact directory.
4. Require `$foreign_cwd/squashfs-root`, then call
   `validate_extracted_appdir` on it.
5. If `APPIMAGE_RUNTIME_SMOKE=1`, call `smoke_extracted_apprun`; otherwise print
   an explicit architecture/static-only message.

`smoke_extracted_apprun "$appdir" "$foreign_cwd" "$log_file"` must:

1. Require `timeout` and `xvfb-run`.
2. Print the AppDir and caller CWD.
3. Create isolated `home`, `config`, `data`, and `cache` directories beneath
   the temporary root so the release smoke cannot read or mutate the runner's
   real OpenHuman state.
4. Launch from `$foreign_cwd` with:

   ```bash
   timeout --signal=TERM --kill-after=5s 15s \
     xvfb-run -a --server-args="-screen 0 1280x960x24" \
     env \
       HOME="$smoke_home" \
       XDG_CONFIG_HOME="$smoke_config" \
       XDG_DATA_HOME="$smoke_data" \
       XDG_CACHE_HOME="$smoke_cache" \
       OPENHUMAN_CEF_PREWARM=0 \
       OPENHUMAN_DISABLE_GPU=1 \
       "$appdir/AppRun"
   ```

   Before `env`, unset `GITHUB_TOKEN`, `GH_TOKEN`,
   `TAURI_SIGNING_PRIVATE_KEY`, `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`,
   `SENTRY_AUTH_TOKEN`, and all `OPENAI_*` variables present in the environment.
   Build the `env -u NAME` arguments from variable **names only**; never print
   values.

5. Capture stdout/stderr in `$log_file` and preserve the timeout status despite
   `set -e`.
6. Treat status `124` as success: the GUI remained alive through the bounded
   startup window. Treat any other status as failure, including an immediate
   clean exit, because it did not prove a live application boundary.
7. Regardless of status, search the log case-insensitively for:
   - `anylinux.so` combined with `cannot be preloaded`;
   - `libxdo.so.3` combined with `cannot open shared object file`;
   - `libcef.so` combined with `cannot open shared object file`;
   - `error while loading shared libraries`.
8. On a forbidden diagnostic or unexpected exit, print the complete smoke log
   and fail. On success, print only lines matching loader/CEF/startup keywords
   plus a message that the application remained alive for the startup window.
   This keeps logs useful without dumping inherited environment data.

### Step 3.3: Call the validator after repack and before signing

In `strip-appimage-graphics-libs.sh`:

1. Define `SCRIPT_DIR` once from `BASH_SOURCE[0]`.
2. Define
   `APPIMAGE_RUNTIME_VALIDATOR="${APPIMAGE_RUNTIME_VALIDATOR:-$SCRIPT_DIR/validate-appimage-runtime.sh}"`.
3. Add `validate_rebuilt_appimage` with the exact one-argument forwarding
   contract above.
4. In `strip_one_appimage`, change the final order to:

   ```bash
   mv "$rebuilt" "$original"
   validate_rebuilt_appimage "$original"
   rm -rf "$workdir"
   MODIFIED_PATHS+=("$original")
   ```

Because `set -e` is active, any static or launch failure stops before
`MODIFIED_PATHS` is populated and before `main` reaches `resign_artifact` or
updater tarball rebuilding.

Keep the intermediate AppDir validation at its current pre-repack location.
It provides a faster diagnostic; the new call proves the repacked bytes.

### Step 3.4: Run GREEN checks

Linux unit/fixture lane:

```bash
bash -n scripts/release/strip-appimage-graphics-libs.sh
bash -n scripts/release/validate-appimage-runtime.sh
bash -n scripts/release/test-strip-appimage-rpaths.sh
bash scripts/release/test-strip-appimage-rpaths.sh
```

Expected GREEN: extracted-layout fixtures cover required libraries, NEEDED
names, RPATHs, equivalent/hard-linked launchers, and post-repack ordering.

Linux real-artifact check, after building an amd64 AppImage:

```bash
APPIMAGE_RUNTIME_SMOKE=1 \
  bash scripts/release/validate-appimage-runtime.sh \
  app/src-tauri/target/x86_64-unknown-linux-gnu/release/bundle/appimage/*.AppImage
```

If the artifact exists under the root target tree instead, use the one exact
matching file under:

```text
target/x86_64-unknown-linux-gnu/release/bundle/appimage/
```

Do not pass multiple glob matches to the one-image CLI; fail the build if the
bundle root unexpectedly contains zero or multiple amd64 AppImages.

macOS:

```bash
bash -n scripts/release/strip-appimage-graphics-libs.sh
bash -n scripts/release/validate-appimage-runtime.sh
bash -n scripts/release/test-strip-appimage-rpaths.sh
bash scripts/release/test-strip-appimage-rpaths.sh
```

Expected: syntax checks pass and the fixture suite skips cleanly without Linux
ELF tooling. Do not attempt to execute the AppImage or claim runtime validation
from macOS.

### Step 3.5: Commit final-artifact validation

```bash
atomic-commit "fix(appimage): validate the repacked runtime before signing" -- \
  scripts/release/validate-appimage-runtime.sh \
  scripts/release/test-strip-appimage-rpaths.sh \
  scripts/release/strip-appimage-graphics-libs.sh
```

---

## Task 4: Enable the amd64 foreign-CWD smoke in the Linux desktop build

**Files:**

- Modify: `.github/workflows/build-desktop.yml`

### Step 4.1: Establish the wiring gap

Run:

```bash
rg -n 'APPIMAGE_RUNTIME_SMOKE|xvfb' .github/workflows/build-desktop.yml
```

Expected RED: the workflow neither installs Xvfb nor enables the final
AppImage smoke.

The behavior itself is already fixture-tested in Task 3. This task is declarative
workflow wiring; validate it with YAML/action checks rather than duplicating the
shell behavior in a YAML text test.

### Step 4.2: Add Linux runtime prerequisites

In `Install Tauri dependencies (ubuntu only)`, add `xvfb` to the existing
`apt-get install -y` list. `timeout` is supplied by Ubuntu's existing
`coreutils`; do not add another timeout package.

Do not add these dependencies to macOS or Windows.

### Step 4.3: Enable smoke only on the reported architecture

Rename the existing step to:

```text
Strip host graphics libs and validate final AppImage
```

Keep its Linux-only `if:` expression, bundle-root arguments, target/profile
variables, and signing secrets unchanged. Add:

```yaml
APPIMAGE_RUNTIME_SMOKE: ${{ matrix.settings.target == 'x86_64-unknown-linux-gnu' && '1' || '0' }}
```

The postprocessor now performs, in order:

1. graphics stripping/interpreter/RPATH/lib.path repair;
2. repack;
3. final artifact extraction and static validation on both Linux architectures;
4. foreign-CWD Xvfb smoke on amd64;
5. signing and updater archive rebuilding.

Do not use `continue-on-error`. Do not change the advisory status of
`pr-quality.yml`; release/staging desktop callers already consume this reusable
workflow and will fail before upload if the validator fails.

Update the step comment to state that:

- final-artifact validation occurs before re-signing;
- amd64 launch is bounded and runs from a foreign CWD;
- arm64 receives static validation in this change;
- updater archives are still rebuilt only after the validated image is final.

### Step 4.4: Validate workflow syntax and formatting

Run whichever actionlint path the repository provides:

```bash
if command -v actionlint >/dev/null 2>&1; then
  actionlint .github/workflows/build-desktop.yml
else
  echo "actionlint unavailable locally; rely on GitHub workflow validation"
fi
```

Then run:

```bash
pnpm exec prettier --check .github/workflows/build-desktop.yml
git diff --check
```

Expected GREEN: actionlint (when installed), Prettier, and whitespace checks
pass.

### Step 4.5: Commit workflow wiring

```bash
atomic-commit "ci(appimage): smoke-test the final amd64 launcher" -- \
  .github/workflows/build-desktop.yml
```

---

## Task 5: Run local checks and obtain Linux evidence

**Files:**

- No new files; verification only.

### Step 5.1: Run the complete macOS-safe local gate

From the worktree root:

```bash
bash -n scripts/release/strip-appimage-graphics-libs.sh
bash -n scripts/release/validate-appimage-runtime.sh
bash -n scripts/release/test-strip-appimage-rpaths.sh
bash scripts/release/test-strip-appimage-rpaths.sh
pnpm exec prettier --check \
  docs/plans/2026-07-24-appimage-sharun-lib-path.md \
  .github/workflows/build-desktop.yml
git diff --check
git status --short
```

Expected on macOS:

- all three shell syntax checks pass;
- the Linux fixture script reports `SKIP`, not `FAIL`, if `patchelf`/ELF is
  unavailable;
- Prettier and `git diff --check` pass;
- status contains no unrelated files.

Do not run `pnpm format`, because it writes across the repository and can mix
unrelated formatting into these atomic commits.

### Step 5.2: Require Linux unit evidence

The existing advisory PR-quality job must run:

```bash
bash -n scripts/release/strip-appimage-graphics-libs.sh
bash scripts/release/test-strip-appimage-rpaths.sh
```

Confirm its log includes passing cases for:

- CI/build path to `+`;
- `+/subdir`, order, deduplication, and idempotency;
- bare relative, absolute CI, traversal, missing-target, and symlink-escape
  rejection;
- hard-linked/equivalent ELF launcher layout;
- anylinux/libxdo/libcef presence;
- NEEDED and RPATH validation.

The job remains advisory by design, but a failure is still a blocker for this
change.

### Step 5.3: Require a real amd64 artifact run

Run the reusable desktop build on an amd64 Ubuntu runner. The log from
`Strip host graphics libs and validate final AppImage` must show:

- final extracted AppDir path;
- caller working directory outside that AppDir;
- normalized `lib.path` containing `+` entries only;
- `shared/bin/OpenHuman` NEEDED entries;
- no CI/build-machine RPATHs;
- presence of `anylinux.so`, `libxdo.so.3`, and `libcef.so`;
- timeout status `124`, documented as the expected live-GUI boundary;
- no preload or loader error for those three libraries;
- signing/updater archive messages only after validation succeeds.

Inspect the later upload step and confirm it consumes the same AppImage path
that was validated. Do not substitute an intermediate `squashfs-root` run for
this evidence.

### Step 5.4: Final history and scope review

Run:

```bash
git status --short
git diff --check HEAD~4..HEAD
git log --oneline -4
git diff --stat baae2b02c..HEAD
git diff --name-only baae2b02c..HEAD
```

Expected:

- four atomic implementation commits after the approved spec;
- only the four implementation paths named at the top of this plan changed;
- no Cargo, AUR, OpenSSL, vendor pin, or PR-quality blocking changes;
- no submodule pointer changes.

If implementation discovers that sharun's bundled version uses a grammar
different from the confirmed newline-plus contract, stop and amend the design
with artifact/source evidence before changing the parser. Do not guess at a
second delimiter grammar in release code.

## Acceptance checklist

- [ ] CI/build paths normalize to `+` or `+/descendant`.
- [ ] `+` is never treated as a delimiter.
- [ ] Output ordering is stable, deduplicated, deterministic, and idempotent.
- [ ] Bare relatives, absolute CI paths, traversal, missing directories,
      separator injection, whitespace ambiguity, and symlink escape fail closed.
- [ ] The released ELF hard-link launcher layout is covered; the shell CWD patch
      remains compatibility-only.
- [ ] Intermediate and final repacked AppDirs both pass library-path,
      required-library, NEEDED, and RPATH validation.
- [ ] The final amd64 AppImage is launched from a foreign CWD under Xvfb and
      remains alive through the bounded startup window.
- [ ] Loader logs contain no `anylinux.so`, `libxdo.so.3`, or `libcef.so`
      resolution error.
- [ ] Validation completes before signing/updater archive rebuilding/upload.
- [ ] Arm64 static validation and all non-Linux workflow behavior remain intact.
- [ ] #3716/OpenSSL, unbundled AUR/core, optional `enigo`, rolling-distro
      matrices, and quick-sharun pinning remain explicit follow-ups.
