# Homebrew Core Submission

This repository currently supports two Homebrew channels:

- `tinyhumansai/openhuman` tap: the existing prebuilt-binary formula used by end users today.
- `homebrew/core` candidate: a source-build formula prepared for submission to Homebrew's official core tap.

The `homebrew/core` candidate lives at [`packages/homebrew-core/openhuman.rb.in`](../packages/homebrew-core/openhuman.rb.in). It is a template, not the final submitted formula, because `homebrew/core` requires a real release tarball checksum for each version.

## Render the candidate formula

After tagging a release:

```bash
bash scripts/release/render-homebrew-core-formula.sh v0.52.27
```

That writes a rendered formula to `packages/homebrew-core/openhuman.rb` using the GitHub source tarball for the tag.

## Local validation

Homebrew uses the checked-out `homebrew/core` repository for local development. To test the candidate formula locally:

```bash
brew update
export HOMEBREW_NO_INSTALL_FROM_API=1
brew tap homebrew/core
cp packages/homebrew-core/openhuman.rb "$(brew --repository homebrew/core)/Formula/o/openhuman.rb"
brew audit --new --formula --strict openhuman
brew install --build-from-source openhuman
brew test openhuman
```

If you need to edit the formula in place:

```bash
brew edit openhuman
```

## Submission checklist

Before opening a PR against `Homebrew/homebrew-core`, confirm:

- The formula builds cleanly from source on supported macOS and Linux environments.
- `brew audit --new --formula --strict openhuman` passes.
- `brew test openhuman` passes.
- The project still meets Homebrew's current `Acceptable Formulae` policy for `homebrew/core`.
- The binary name and install layout are intentional. Right now the formula installs `openhuman-core` and adds an `openhuman` symlink for user ergonomics.

## Known follow-up items

- Linux dependencies may need to be tightened if Homebrew CI reports missing native libraries beyond `openssl@3`.
- If Homebrew reviewers object to the `openhuman` symlink, the upstream binary name should be renamed directly in `Cargo.toml` instead of relying on formula-level aliasing.
- If the build remains too broad for `homebrew/core`, split the CLI surface from desktop-only integrations so the formula builds with fewer native dependencies.
