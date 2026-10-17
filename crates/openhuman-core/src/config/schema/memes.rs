//! `[memes]` — tuning for TinyMemes reply remixing.
//!
//! Whether remixing runs at all is `[components] memes`; which threads are in
//! the treatment arm is the `OPENHUMAN_TINYMEMES` flag (`off`, `on`, `ab`,
//! `ab:NN`, with `OPENHUMAN_TINYMEMES_DEFAULT` as the build-time default). The
//! environment variables below stay as operator overrides and win over this
//! section when set:
//!
//! | setting | override |
//! | --- | --- |
//! | `timeout_ms` | `OPENHUMAN_TINYMEMES_TIMEOUT_MS` |
//! | `meme_cooldown` | `OPENHUMAN_TINYMEMES_MEME_COOLDOWN` |

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const DEFAULT_TIMEOUT_MS: u64 = 20_000;
pub const DEFAULT_MEME_COOLDOWN: u32 = 3;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(default)]
pub struct MemesConfig {
    /// Time budget for one remix, in milliseconds. The original reply is
    /// delivered when it runs out.
    pub timeout_ms: u64,
    /// Replies between memes.
    pub meme_cooldown: u32,
}

impl Default for MemesConfig {
    fn default() -> Self {
        Self {
            timeout_ms: DEFAULT_TIMEOUT_MS,
            meme_cooldown: DEFAULT_MEME_COOLDOWN,
        }
    }
}

#[cfg(test)]
#[path = "memes_tests.rs"]
mod tests;
