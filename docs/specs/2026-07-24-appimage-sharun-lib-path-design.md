# AppImage sharun library-path repair

## Context

Issue #5037 reports that Linux AppImages fail when launched outside their
extracted AppDir. The launcher logs that `anylinux.so` cannot be preloaded and
then fails to resolve `libxdo.so.3`, even though both libraries are bundled.
Running the extracted application with an explicit AppDir-rooted
`LD_LIBRARY_PATH` succeeds.

Read-only analysis verified the final v0.61.2 and v0.63.1 amd64 AppImages:

- `AppRun`, `sharun`, and `bin/OpenHuman` are hard links to the sharun ELF
  launcher.
- The real application is `shared/bin/OpenHuman`.
- `lib/` contains `anylinux.so`, `libxdo.so.3`, `libcef.so`, `libssl.so.3`, and
  `libcrypto.so.3`; `shared/lib` is a symlink to `../lib`.
- `shared/lib/lib.path` contains the bare relative entry `shared/lib`.
- sharun 2.2.4 and 2.2.5 expand the canonical `+` marker to an absolute path
  rooted at the AppDir's `lib/` directory. They do not anchor a bare
  `shared/lib` entry.

The release postprocessor causes the malformed entry.
`rewrite_sharun_lib_path` converts absolute CI paths beneath
`squashfs-root/shared/lib` into `shared/lib`. The intended compensation,
`patch_apprun_sharun_cwd`, only edits a shell `AppRun` and intentionally skips
the shipped ELF `AppRun`.

Consequently, `shared/lib` resolves from the caller's current directory. It
works when the caller happens to be the AppDir and fails from locations such as
`~/Downloads` or `/tmp`.

## Evidence boundary

Artifact hashes, SquashFS layouts, ELF metadata, release-script behavior, sharun
path expansion, and caller-directory resolution were verified directly.

This development host is macOS, and its Docker daemon is unavailable, so it
cannot execute the Linux ELF launcher. An identical v0.63.1 process failure is a
strong inference from the verified path semantics and must be confirmed by a
Linux CI smoke test.

## Goals

- Write sharun-native, AppDir-anchored entries to `shared/lib/lib.path`.
- Reject malformed bare-relative or CI-runner paths before an AppImage ships.
- Cover the actual ELF `AppRun` layout used by released artifacts.
- Validate the final repacked AppImage, rather than only the intermediate
  AppDir.
- Exercise the final launcher from a working directory outside the AppDir on
  Linux.
- Preserve the existing graphics-library stripping, interpreter repair,
  signing, updater archive, and multi-architecture behavior.

## Non-goals

- Making the unbundled AUR/core binary independent of system `libxdo`.
- Changing the unconditional `enigo` dependency or voice input behavior.
- Replacing bundled OpenSSL or claiming to resolve issue #3716's later-stage
  symbol-skew/segfault report.
- Bypassing sharun with a custom `LD_LIBRARY_PATH` launcher.
- Pinning mutable quick-sharun downloads; that is a separate supply-chain
  follow-up.
- Converting the existing PR-quality AppImage job from advisory to blocking.

## Considered approaches

### 1. Emit sharun-native `+` paths and validate them

Rewrite AppDir library roots to sharun's canonical `+` representation. A root
library directory becomes `+`; any retained directory beneath it becomes a
`+/subdir` entry. Preserve the format's entry separator without treating the
marker itself as a separator.

Validation rejects:

- absolute CI runner paths;
- bare relative entries such as `shared/lib`;
- traversal components;
- AppDir-relative targets that do not exist.

This directly restores sharun's intended absolute path expansion and is the
selected approach.

### 2. Change the working directory in `AppRun.sh`

Patching the shipped shell wrapper or setting a sharun working-directory
variable could make `shared/lib` resolve by accident. It leaves malformed
library-path semantics in place, depends on launcher shape, and does not explain
the existing ELF `AppRun` skip clearly enough to be the primary fix.

### 3. Bypass sharun

A wrapper could execute `shared/bin/OpenHuman` with an explicit absolute
`LD_LIBRARY_PATH`. This duplicates sharun's loader and portability behavior and
risks losing its environment and compatibility hooks.

## Design

### Library-path normalization

Refactor `rewrite_sharun_lib_path` in
`scripts/release/strip-appimage-graphics-libs.sh` so CI-origin entries are
normalized into sharun-native AppDir library entries.

The normalizer must:

1. Parse entries without destroying the `+` marker.
2. Recognize CI paths rooted under an extracted `squashfs-root` or the existing
   build `data` layout.
3. Map the AppDir's canonical library root to `+`, and valid descendants to
   `+/relative/subdir`.
4. Deduplicate entries while preserving stable order.
5. Exclude unrelated absolute host paths.
6. Fail instead of falling back to a bare `shared/lib` entry when no valid
   AppDir library root can be derived.
7. Be idempotent when the file already contains valid sharun-native entries.

The implementation must follow sharun's actual `lib.path` delimiter grammar as
confirmed from the bundled sharun version. Tests must pin the accepted textual
forms rather than rely on an informal reconstruction.

### Fail-closed validation

Extend `validate_sharun_lib_path` to reject any non-empty entry that is:

- a CI/build-machine absolute path;
- a bare relative path not beginning with the sharun marker;
- a traversal outside the bundled library root;
- a marker-relative directory absent from the AppDir.

The validation error must identify the offending entry and refuse to repack or
publish the artifact.

`patch_apprun_sharun_cwd` may remain as compatibility hardening for historical
shell AppRun layouts, but the new tests must establish that an ELF AppRun does
not need this patch once `lib.path` is valid. Comments that describe changing
the working directory as the primary fix must be updated to reflect that role.

### Regression tests

Extend `scripts/release/test-strip-appimage-rpaths.sh` with isolated cases for:

- a CI absolute `.../squashfs-root/shared/lib` entry rewriting to the canonical
  marker;
- valid marker/subdirectory entries and stable ordering;
- deduplication and idempotency;
- rejection of bare `shared/lib`, CI paths, traversal, and missing targets;
- the released launcher layout where ELF `AppRun`, `sharun`, and
  `bin/OpenHuman` are hard-linked or equivalent launcher entries;
- expansion of the rewritten value according to sharun's algorithm, proving it
  resolves beneath the fixture AppDir regardless of the caller's directory.

The test remains Linux-only because it needs ELF fixtures and `patchelf`; it
continues to skip cleanly on unsupported development hosts.

### Final-artifact validation and smoke

After postprocessing and repacking, Linux CI must inspect the final `.AppImage`
that will be uploaded:

1. Extract it into a temporary directory.
2. Re-run the sharun `lib.path`, required-library, ELF `NEEDED`, and RPATH
   validations against the extracted final layout.
3. Change to an unrelated temporary working directory.
4. Launch the extracted `AppRun` under a bounded timeout and the existing
   headless/Xvfb facilities.
5. Fail on preload or loader diagnostics for `anylinux.so`, `libxdo.so.3`, or
   `libcef.so`.
6. Treat reaching the normal application initialization boundary as success;
   the smoke does not require an interactive desktop session.

The smoke belongs in the Linux desktop build path after
`Strip host graphics libs from AppImage`, so it exercises the same final
artifact that later signing/upload steps consume. It must cover amd64 first,
where #5037 was reproduced, while keeping the validator architecture-neutral so
arm64 can use it when an executable runner is available.

## Failure behavior

An invalid `lib.path`, missing bundled runtime library, failed final extraction,
or foreign-CWD loader error fails the Linux artifact build before signing or
upload. The release must not continue with a warning-only malformed launcher.

The smoke must print the extracted path, caller working directory, normalized
`lib.path`, and relevant loader diagnostics without leaking credentials.

## Validation

Local/macOS:

- `bash -n scripts/release/strip-appimage-graphics-libs.sh`
- the regression script skips cleanly when Linux ELF tooling is unavailable;
- `git diff --check` and repository formatting checks.

Linux:

- run `scripts/release/test-strip-appimage-rpaths.sh` with `patchelf`;
- run the final-artifact validator against a freshly built amd64 AppImage;
- launch from a foreign working directory under timeout/Xvfb;
- confirm no `anylinux.so`, `libxdo.so.3`, or `libcef.so` loader errors;
- confirm the updater archive/signing flow still consumes the validated
  postprocessed artifact.

## Acceptance criteria

- Final `lib.path` uses only valid sharun-native AppDir entries.
- Bare `shared/lib` and CI absolute entries fail validation.
- Rewriting is deterministic and idempotent.
- Released-style ELF AppRun fixtures are covered.
- The final repacked amd64 AppImage launches from outside its AppDir without the
  #5037 preload/library-resolution errors.
- Required bundled libraries, ELF dependencies, and RPATHs remain valid.
- Existing graphics stripping, signing, updater archive, and non-Linux build
  behavior remain unchanged.
