#!/usr/bin/env node
// Shim so Gradle's Rust-build task (buildSrc/BuildTask.kt: `node tauri android
// android-studio-script`) can find the tauri CLI. tauri-cli's own `android init`
// is supposed to leave this in place, but under pnpm's shell-wrapper .bin shims
// (rather than npm's direct symlink-to-JS), that step doesn't produce it. This
// just forwards into the real CLI entry resolved the normal node_modules way.
// ESM because this directory's package.json sets "type": "module".
await import("@tauri-apps/cli/tauri.js");
