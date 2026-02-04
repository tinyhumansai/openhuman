//! Custom V8 module resolver and loader for `@alphahuman/*` imports.
//!
//! NOTE: Currently unused. Skills access bridge APIs via globals (db, store, console)
//! injected by v8_skill_instance.rs. This module is reserved for future ES module
//! import support (e.g., `import { db } from '@alphahuman/db'`).
//!
//! The globals-based approach was chosen because:
//! - V8/deno_core shares the module loader across all contexts in the same runtime
//! - Per-skill module loaders would require separate runtime instances
//! - Globals are simpler and sufficient for the initial implementation
