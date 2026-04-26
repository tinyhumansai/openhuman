# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **DevOps**: Added Sentry debug symbol upload to CI/CD pipeline
  - Rust debug symbols from Tauri build are now automatically uploaded to Sentry on every main branch push
  - Creates and finalizes Sentry releases with proper version tagging (`openhuman@{version}`)
  - Enables accurate stack trace symbolication for production releases
  - Added `scripts/upload_sentry_symbols.sh` helper script for local symbol uploads

### Changed

- **CI**: Updated `build.yml` workflow to upload debug symbols after successful builds
  - Symbol upload only triggers on main branch pushes (not PRs)
  - Added `actions: read` permission for Sentry commit association

### Dependencies

- None

### Fixed

- None

---

## [0.52.28] - 2026-04-18

See [release notes](https://github.com/tinyhumansai/openhuman/releases/tag/v0.52.28) for details.