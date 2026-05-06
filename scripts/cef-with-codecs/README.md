# Building CEF with Proprietary Codecs

Tracks issue #1223 — vendored CEF lacks H.264 / AAC support so Google Meet's
dynamic (video) virtual backgrounds, embedded YouTube/Vimeo previews, and
any HTML5 `<video>` source pulling H.264-in-MP4 fail with
`MEDIA_ERR_SRC_NOT_SUPPORTED: PipelineStatus::DEMUXER_ERROR_NO_SUPPORTED_STREAMS:
FFmpegDemuxer: no supported streams`. Empirical confirmation of the codec
absence is in #1223 and in [`feedback_cef_runtime_gaps.md`](https://github.com/tinyhumansai/openhuman/issues/1223#issuecomment-4379209818)
gap #3.

The Spotify CDN (`cef-builds.spotifycdn.com`) — which `download-cef` and
all other public CEF wrappers default to — ships **only** open-source
codecs. Every flavor (`standard`, `minimal`, `client`, `tools`, `*_symbols`)
is built with `proprietary_codecs = false`. To get H.264 / AAC support
into the embedded webview we have to compile CEF ourselves with
Chrome-branded FFmpeg and host the resulting binary somewhere our build
script can fetch it from.

This directory is the build infrastructure for that: scripts that drive
the upstream `automate-git.py` toolchain, a local install helper that
drops the result into `CEF_PATH` so `cargo build` picks it up, and the
license posture / hosting documentation.

> **The actual built binary is NOT committed to this repo and never will
> be.** It is multiple gigabytes and carries license obligations (see
> below). Hosting + distribution is a separate operational concern.

---

## License posture (READ BEFORE BUILDING)

H.264 / AVC carries patent obligations under the
[MPEG-LA AVC Patent Portfolio License](https://www.mpegla.com/programs/avc-h-264/).
Bundling an H.264 decoder into a redistributed application can require
royalty payments depending on:

- distribution model (free vs paid),
- annual end-user count,
- whether the decoder is hardware-accelerated by the OS (some royalty
  carve-outs apply for "system supplied" decoders),
- jurisdiction.

Browsers like Firefox sidestep this by downloading Cisco's OpenH264 binary
plugin at runtime — Cisco pays the royalties on their users' behalf. CEF
does not currently ship that plugin path.

**Before running this build, get sign-off from legal / business** on:

1. Whether the AVC license fee is in budget for OpenHuman's distribution
   channels (desktop installer, GitHub releases, app stores).
2. Whether the AAC patent pool (separate licensor) is also in scope —
   AAC is bundled with H.264 in the same `proprietary_codecs = true`
   build flag, so you cannot have one without the other.
3. Whether HEVC / H.265 should also be enabled (separate flag,
   `enable_hevc_parser_and_hw_decoder = true`, which has its own MPEG-LA
   pool).

If the answer to (1) or (2) is no, **stop here**. The honest fallback is
to surface "dynamic backgrounds not supported" in the Effects picker UI
(see #1223 path D) rather than ship without a license.

---

## Build inputs

| Variable | Value (CEF 146 line) |
|---|---|
| Target CEF version | `146.0.9+g3ca6a87+chromium-146.0.7680.165` (matches `cef = "=146.4.1"` in `app/src-tauri/Cargo.toml`) |
| Chromium branch | `7680` |
| GN args added | `proprietary_codecs=true ffmpeg_branding="Chrome"` |
| Required GN args (already implied) | `is_official_build=true` (release builds only) |
| Optional HEVC extension | `enable_hevc_parser_and_hw_decoder=true` (separate license) |
| Build platforms | macOS arm64 + x86_64, Linux arm64 + x86_64, Windows x86_64 |
| Disk required | ~150 GB per platform (Chromium source + build cache) |
| Wall-clock | ~2-4 hours per platform on M2/M3 Mac, longer on Linux/Windows |
| Output artifact | `cef_binary_<ver>_<platform>_minimal.tar.bz2` |

The `minimal` flavor is what `download-cef` already targets (matches
`pub fn minimal()` selection in `download_cef::CefVersion`). Skipping
the `standard` flavor saves ~200 MB per artifact and the sample apps
aren't shipped to users.

---

## Build host requirements (per upstream CEF docs)

Per [CEF Automated Build Setup](https://bitbucket.org/chromiumembedded/cef/wiki/AutomatedBuildSetup.md):

- **macOS**: Xcode + macOS SDK matching the target Chromium milestone.
- **Linux**: Ubuntu 22.04 LTS recommended; needs `clang`, `lld`,
  `libstdc++-12-dev`, plus the chromium `install-build-deps.sh` package
  set.
- **Windows**: Visual Studio 2022 with the C++ workload, Windows 11 SDK.

All platforms: Python 3, Git, `depot_tools` (the script will pull a
fresh copy if `--depot-tools-dir` doesn't exist).

---

## Quick start (single platform, local dev)

> **Prerequisites:** the build-host requirements above, plus the legal
> sign-off documented in the license-posture section.

```bash
# 1. Run the build (2-4 hours on M2/M3 Mac).
#    Output lands at $CEF_BUILD_DIR/chromium/src/cef/binary_distrib/
./scripts/cef-with-codecs/build-cef-with-codecs.sh

# 2. Extract the resulting tarball to the cache that tauri-cef expects
#    so cargo build picks it up via the existing CEF_PATH wiring.
./scripts/cef-with-codecs/install-local.sh

# 3. Verify the codec gates inside dev:app:
pnpm dev:app
# In a second terminal once a webview is loaded:
node app/src-tauri/scripts/diagnose-cef-runtime.mjs probe   # path may vary
# Expect h264_baseline / h264_main / h264_high / aac_lc → true
```

If `h264_baseline` returns `true` after step 3, the codec build is
correctly installed. Re-run #1053 Phase B smoke (Gmeet → Effects → pick
a dynamic / video background) to confirm the original symptom is gone.

---

## CI / shared distribution (out of scope for this PR)

The build script alone is enough for an individual developer to validate
the fix end-to-end. To ship the binary to all developers + release builds
without each machine needing a 4-hour compile:

1. Run the build on a powerful CI runner (GitHub Actions self-hosted, or
   a beefy on-prem box).
2. Upload the resulting `cef_binary_*.tar.bz2` to a private CDN
   (`s3://openhuman-cef-builds/<version>/<platform>/...` or equivalent).
3. Set `CEF_DOWNLOAD_URL` in `scripts/load-dotenv.sh` (or as a
   per-developer env override) to point at that CDN.
4. The vendored `download-cef` crate will fetch from the new URL on
   first build, just like it currently does from Spotify CDN.

Tracking this hosting work as a follow-up issue once the legal sign-off
comes back. The build script in this PR is the upstream half of that
pipeline.

---

## What this PR does not do

- **Does not compile any binary.** The build is too long + too disk-heavy
  to run in CI on every PR. Maintainers run the script offline.
- **Does not host any binary.** That belongs to the CDN follow-up above.
- **Does not flip any default in `download-cef`.** Spotify CDN remains the
  default until the legal review + private CDN are in place.
- **Does not enable HEVC.** That's a separate license pool; revisit if
  there's a concrete user-visible feature blocked on HEVC.
- **Does not change vendored `tauri-cef`.** No submodule pin bump, no
  source-tree edits — the upstream crate already handles `CEF_PATH` /
  `CEF_DOWNLOAD_URL` overrides.

The only files this PR touches are this README, the two helper scripts,
and the issue tracker (#1223 cross-link).

---

## Related

- Issue #1223 — bug: dynamic Gmeet backgrounds fail on H.264 demux.
- Issue #1053 — parent: Gmeet bg effects (this is the codec follow-up).
- PR #1222 — surfaced + diagnosed the codec gap during gmeet routing-
  reliability work; harness `scripts/diagnose-cef-runtime.mjs` added there.
- Memory `feedback_cef_runtime_gaps.md` — gap #3 reclassified, codec gap
  documented with full diagnostic procedure.
