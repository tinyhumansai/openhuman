//! Cached cell-wrapped blocks and indexed viewport lookup.
use super::state::{EntryKind, TranscriptState};
use super::theme::safe_text;
use std::collections::BTreeSet;
use unicode_width::UnicodeWidthChar;

#[derive(Default)]
pub struct ViewportCache {
    width: u16,
    blocks: Vec<CachedBlock>,
    heights: HeightIndex,
    resident: BTreeSet<usize>,
    revision: u64,
    height: u16,
    offset: usize,
    anchor: Option<(usize, usize)>,
    #[cfg(test)]
    recomputed: usize,
}
struct CachedBlock {
    revision: u64,
    height: usize,
    rows: Vec<String>,
}

#[derive(Clone)]
pub struct VisibleRow {
    pub text: String,
    pub entry: usize,
    pub first: bool,
    pub kind: EntryKind,
}

/// Dynamic Fenwick tree: append and point updates preserve logarithmic row lookup.
#[derive(Default)]
struct HeightIndex {
    tree: Vec<usize>,
    total: usize,
}
impl HeightIndex {
    fn prefix(&self, mut count: usize) -> usize {
        let mut sum = 0;
        while count > 0 {
            sum += self.tree[count - 1];
            count &= count - 1;
        }
        sum
    }
    fn push(&mut self, height: usize) {
        let index = self.tree.len() + 1;
        let start = index - index.isolate_lowest_one();
        self.tree.push(height + self.total - self.prefix(start));
        self.total += height;
    }
    fn update(&mut self, index: usize, before: usize, after: usize) {
        self.total = self.total - before + after;
        let mut index = index + 1;
        while index <= self.tree.len() {
            self.tree[index - 1] = self.tree[index - 1] - before + after;
            index += index.isolate_lowest_one();
        }
    }
    fn containing(&self, row: usize) -> usize {
        let mut index = 0;
        let mut sum = 0;
        let mut bit = self.tree.len().checked_next_power_of_two().unwrap_or(0);
        while bit != 0 {
            let next = index + bit;
            if next <= self.tree.len() && sum + self.tree[next - 1] <= row {
                sum += self.tree[next - 1];
                index = next;
            }
            bit >>= 1;
        }
        index
    }
}

impl ViewportCache {
    pub fn rows(
        &mut self,
        state: &TranscriptState,
        width: u16,
        height: u16,
        offset: usize,
    ) -> (Vec<VisibleRow>, usize) {
        let width = width.max(1);
        let changes = state.viewport_changes_since(self.revision);
        let anchor = if offset > 0
            && offset == self.offset
            && changes.is_some()
            && (self.width != width || self.height != height)
        {
            self.anchor
        } else {
            None
        };
        if self.width != width || changes.is_none() || self.blocks.len() > state.entries().len() {
            self.blocks.clear();
            self.heights = HeightIndex::default();
            self.resident.clear();
            self.width = width;
        }
        #[cfg(test)]
        {
            self.recomputed = 0;
        }
        for index in changes.unwrap_or_default() {
            if index < self.blocks.len() {
                let entry = &state.entries()[index];
                if self.blocks[index].revision == entry.revision {
                    continue;
                }
                let rows = entry_rows(entry, width);
                self.heights
                    .update(index, self.blocks[index].height, rows.len());
                self.blocks[index] = CachedBlock {
                    revision: entry.revision,
                    height: rows.len(),
                    rows,
                };
                self.resident.insert(index);
                #[cfg(test)]
                {
                    self.recomputed += 1;
                }
            }
        }
        for entry in &state.entries()[self.blocks.len()..] {
            let rows = entry_rows(entry, width);
            let block = CachedBlock {
                revision: entry.revision,
                height: rows.len(),
                rows,
            };
            self.heights.push(block.height);
            self.resident.insert(self.blocks.len());
            self.blocks.push(block);
            #[cfg(test)]
            {
                self.recomputed += 1;
            }
        }
        self.revision = state.viewport_revision();
        let max_scroll = self.heights.total.saturating_sub(height as usize);
        let top = anchor
            .filter(|(entry, _)| *entry < self.blocks.len())
            .map(|(entry, line)| {
                self.heights.prefix(entry) + line.min(self.blocks[entry].height.saturating_sub(1))
            })
            .unwrap_or_else(|| max_scroll.saturating_sub(offset.min(max_scroll)))
            .min(max_scroll);
        self.offset = max_scroll - top;
        self.height = height;
        let mut index = self.heights.containing(top);
        let first_visible = index;
        let mut absolute = self.heights.prefix(index);
        self.anchor = (index < self.blocks.len()).then_some((index, top - absolute));
        let mut rows = Vec::with_capacity(height as usize);
        while index < self.blocks.len() && rows.len() < height as usize {
            if self.blocks[index].rows.is_empty() {
                self.blocks[index].rows = entry_rows(&state.entries()[index], width);
                self.resident.insert(index);
            }
            for (line, text) in self.blocks[index]
                .rows
                .iter()
                .enumerate()
                .skip(top.saturating_sub(absolute))
                .take(height as usize - rows.len())
            {
                rows.push(VisibleRow {
                    text: text.clone(),
                    entry: index,
                    first: line == 0,
                    kind: state.entries()[index].kind,
                });
            }
            absolute += self.blocks[index].height;
            index += 1;
        }
        let stale: Vec<_> = self
            .resident
            .iter()
            .copied()
            .filter(|index| {
                *index < first_visible.saturating_sub(64) || *index > first_visible + 128
            })
            .collect();
        for index in stale {
            self.blocks[index].rows = Vec::new();
            self.resident.remove(&index);
        }
        (rows, max_scroll)
    }
    pub fn total_rows(&self) -> usize {
        self.heights.total
    }
    pub fn resolved_offset(&self) -> usize {
        self.offset
    }
}

fn entry_rows(entry: &super::state::Entry, width: u16) -> Vec<String> {
    let text = if entry.kind == EntryKind::Thinking && !entry.expanded {
        "Reasoning · click to expand".to_string()
    } else if let Some(activity) = &entry.activity {
        if entry.expanded {
            format!(
                "{}\n{}",
                activity.summary(),
                activity
                    .details()
                    .lines()
                    .take(20)
                    .collect::<Vec<_>>()
                    .join("\n")
            )
        } else {
            activity.summary()
        }
    } else {
        entry.text.clone()
    };
    let mut rows = wrap_cells(&safe_text(&text), width as usize);
    if matches!(entry.kind, EntryKind::User | EntryKind::Assistant) {
        rows.insert(
            0,
            if entry.kind == EntryKind::User {
                "You".into()
            } else {
                "OpenHuman".into()
            },
        );
    }
    if entry.activity.is_some() && entry.expanded {
        rows.push("[Open /tools for stored output details]".into());
    }
    rows.push(String::new());
    rows
}

/// Wrap by terminal cells without splitting UTF-8 or counting combining marks.
pub fn wrap_cells(value: &str, width: usize) -> Vec<String> {
    let width = width.max(1);
    let mut rows = vec![String::new()];
    let mut used = 0;
    for ch in value.chars() {
        if ch == '\n' {
            rows.push(String::new());
            used = 0;
            continue;
        }
        let cells = ch.width().unwrap_or(0);
        if cells > 0 && used + cells > width && used > 0 {
            rows.push(String::new());
            used = 0;
        }
        rows.last_mut().unwrap().push(ch);
        used += cells;
    }
    rows
}

#[cfg(test)]
#[path = "viewport_tests.rs"]
mod tests;
