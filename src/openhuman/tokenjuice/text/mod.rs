//! Text-processing utilities for the TokenJuice engine.

pub mod ansi;
pub mod process;
pub mod width;

pub use ansi::strip_ansi;
pub use process::{
    clamp_text, clamp_text_middle, dedupe_adjacent, head_tail, normalize_lines, pluralize,
    trim_empty_edges,
};
pub use width::{count_terminal_cells, count_text_chars, graphemes};
