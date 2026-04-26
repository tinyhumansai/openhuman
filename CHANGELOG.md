# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- **Config (Issue #933)**: Bootstrap from config.toml RPC URL with runtime derivation
  - Added "Configure RPC URL" option on Welcome screen for self-hosted/internal deployments
  - Users can now set core JSON-RPC URL on login screen without build-time configuration
  - RPC URL persisted to localStorage and restored on next launch
  - Added "Test Connection" button to validate RPC endpoint before saving
  - Core now exposes `openhuman.config_get_client_config` RPC method returning safe client fields
  - Frontend `coreRpcClient` respects user-configured URL over build-time defaults
  - Unit tests added for URL persistence and validation utilities

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

- **Webview Accounts**: Verified loading overlay implementation (Issue #867)
  - Webviews now display a loading spinner while CEF initializes provider pages
  - Three independent signals trigger reveal: native `on_page_load`, CDP `Page.loadEventFired`, and 15s watchdog
  - Webview spawns at 1x1 size (off-screen) to prevent blank coverage during load
  - Rust backend resizes/repositions webview and emits `webview-account:load` event
  - Frontend dispatches status='open' to hide spinner once page is painted

---

## [0.52.28] - 2026-04-18

See [release notes](https://github.com/tinyhumansai/openhuman/releases/tag/v0.52.28) for details.

## [0.52.27] - 2026-04-16

### Added

- **Skills**: Enhanced skill creation with code generation and GitHub integration
  - New skill templates with intelligent defaults
  - Improved skill editor with syntax highlighting
  - GitHub sync for skill versioning

## [0.52.26] - 2026-04-14

### Fixed

- **Conversations**: Fixed thread pagination and loading states
  - Improved infinite scroll behavior
  - Added proper loading indicators
  - Resolved race conditions in message fetching
