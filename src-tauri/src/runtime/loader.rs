//! Custom module resolver and loader for `@openhuman/*` imports.
//!
//! NOTE: Currently unused. Skills access bridge APIs via globals (db, store, console)
//! injected by qjs_skill_instance.rs. This module is reserved for future ES module
//! import support (e.g., `import { db } from '@openhuman/db'`).
//!
//! The globals-based approach was chosen because:
//! - Globals are simpler and sufficient for the initial implementation
//! - Per-skill module loaders can be added later if needed
