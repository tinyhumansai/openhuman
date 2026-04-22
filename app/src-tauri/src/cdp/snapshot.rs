//! Generic wrapper around `DOMSnapshot.captureSnapshot`. Parses the
//! flat-array node tree CDP returns into indexable helpers each provider
//! can use to extract chat / channel / message rows without executing any
//! page JavaScript.
//!
//! The raw CDP response is a pair of parallel arrays keyed by node index:
//!   * `parentIndex[i]` — parent node index (-1 for roots)
//!   * `nodeType[i]`    — 1 = element, 3 = text, etc.
//!   * `nodeName[i]`    — index into `strings` (element tag name)
//!   * `nodeValue[i]`   — index into `strings` (text content for text nodes)
//!   * `attributes[i]`  — flat `[nameIdx, valueIdx, …]` string-table indices
//!
//! `Snapshot` owns these arrays plus a lazily-computed children map so
//! subtree walks are O(subtree) instead of O(total).

use serde::Deserialize;
use serde_json::json;

use super::CdpConn;

pub const NODE_TYPE_ELEMENT: i32 = 1;
pub const NODE_TYPE_TEXT: i32 = 3;

#[derive(Deserialize, Debug, Default)]
struct CaptureSnapshot {
    #[serde(default)]
    documents: Vec<DocumentSnap>,
    #[serde(default)]
    strings: Vec<String>,
}

#[derive(Deserialize, Debug, Default)]
struct DocumentSnap {
    #[serde(default)]
    nodes: NodeTreeSnap,
}

#[derive(Deserialize, Debug, Default)]
struct NodeTreeSnap {
    #[serde(rename = "parentIndex", default)]
    parent_index: Vec<i32>,
    #[serde(rename = "nodeType", default)]
    node_type: Vec<i32>,
    #[serde(rename = "nodeName", default)]
    node_name: Vec<i32>,
    #[serde(rename = "nodeValue", default)]
    node_value: Vec<i32>,
    #[serde(default)]
    attributes: Vec<Vec<i32>>,
}

pub struct Snapshot {
    strings: Vec<String>,
    nodes: NodeTreeSnap,
    children: Vec<Vec<usize>>,
}

impl Snapshot {
    /// Run `DOMSnapshot.captureSnapshot` on an attached session and return
    /// the parsed main-document tree. Iframes are ignored — none of the
    /// migrated providers render chat lists inside iframes.
    pub async fn capture(cdp: &mut CdpConn, session: &str) -> Result<Self, String> {
        let raw = cdp
            .call(
                "DOMSnapshot.captureSnapshot",
                json!({
                    "computedStyles": [],
                    "includePaintOrder": false,
                    "includeDOMRects": false,
                }),
                Some(session),
            )
            .await?;
        let snap: CaptureSnapshot =
            serde_json::from_value(raw).map_err(|e| format!("decode DOMSnapshot: {e}"))?;
        let strings = snap.strings;
        let nodes = snap
            .documents
            .into_iter()
            .next()
            .map(|d| d.nodes)
            .unwrap_or_default();
        let count = nodes.node_type.len();
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
        for (i, &p) in nodes.parent_index.iter().enumerate() {
            if p >= 0 && (p as usize) < count {
                children[p as usize].push(i);
            }
        }
        Ok(Self {
            strings,
            nodes,
            children,
        })
    }

    pub fn len(&self) -> usize {
        self.nodes.node_type.len()
    }

    pub fn node_type(&self, idx: usize) -> i32 {
        self.nodes.node_type.get(idx).copied().unwrap_or(0)
    }

    pub fn is_element(&self, idx: usize) -> bool {
        self.node_type(idx) == NODE_TYPE_ELEMENT
    }

    pub fn tag(&self, idx: usize) -> &str {
        self.str_at(*self.nodes.node_name.get(idx).unwrap_or(&-1))
    }

    pub fn text_value(&self, idx: usize) -> &str {
        self.str_at(*self.nodes.node_value.get(idx).unwrap_or(&-1))
    }

    pub fn attr(&self, idx: usize, name: &str) -> Option<&str> {
        let flat = self.nodes.attributes.get(idx)?;
        let mut i = 0;
        while i + 1 < flat.len() {
            if self.str_at(flat[i]) == name {
                return Some(self.str_at(flat[i + 1]));
            }
            i += 2;
        }
        None
    }

    /// Classes split on whitespace. Empty for elements with no `class` attr.
    pub fn classes(&self, idx: usize) -> impl Iterator<Item = &str> {
        self.attr(idx, "class").unwrap_or("").split_whitespace()
    }

    pub fn has_class(&self, idx: usize, name: &str) -> bool {
        self.classes(idx).any(|c| c == name)
    }

    /// Discord renders hashed class names (e.g. `name__abcde`). Callers
    /// check for the unhashed prefix.
    pub fn class_starts_with(&self, idx: usize, prefix: &str) -> bool {
        self.classes(idx).any(|c| c.starts_with(prefix))
    }

    pub fn children(&self, idx: usize) -> &[usize] {
        self.children.get(idx).map(|v| v.as_slice()).unwrap_or(&[])
    }

    /// Depth-first pre-order walk of every descendant of `root` (including
    /// `root` itself). Cheap enough for chat-list scrapes that run every
    /// 2 seconds — DOM has thousands of nodes, not millions.
    pub fn descendants(&self, root: usize) -> Vec<usize> {
        let mut out = Vec::new();
        let mut stack = vec![root];
        while let Some(idx) = stack.pop() {
            out.push(idx);
            for &k in self.children(idx).iter().rev() {
                stack.push(k);
            }
        }
        out
    }

    /// Concatenate every TEXT_NODE under `root` in document order. Runs of
    /// whitespace collapse to a single space and the result is trimmed.
    pub fn text_content(&self, root: usize) -> String {
        let mut out = String::new();
        for idx in self.descendants(root) {
            if self.node_type(idx) == NODE_TYPE_TEXT {
                out.push_str(self.text_value(idx));
            }
        }
        collapse_ws(&out)
    }

    /// First descendant (or `root` itself) matching `pred`. Depth-first.
    pub fn find_descendant<F>(&self, root: usize, pred: F) -> Option<usize>
    where
        F: Fn(&Snapshot, usize) -> bool,
    {
        self.descendants(root).into_iter().find(|&i| pred(self, i))
    }

    /// Every element (anywhere in the document) matching `pred`. Returned
    /// in document order.
    pub fn find_all<F>(&self, pred: F) -> Vec<usize>
    where
        F: Fn(&Snapshot, usize) -> bool,
    {
        let mut out = Vec::new();
        for i in 0..self.len() {
            if self.is_element(i) && pred(self, i) {
                out.push(i);
            }
        }
        out
    }

    fn str_at(&self, idx: i32) -> &str {
        if idx < 0 {
            return "";
        }
        self.strings
            .get(idx as usize)
            .map(String::as_str)
            .unwrap_or("")
    }
}

fn collapse_ws(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut last_space = true;
    for ch in s.chars() {
        if ch.is_whitespace() {
            if !last_space {
                out.push(' ');
                last_space = true;
            }
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim().to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn collapse_ws_collapses_and_trims() {
        assert_eq!(collapse_ws("  hello   world  "), "hello world");
        assert_eq!(collapse_ws("\n\tfoo\n\n"), "foo");
        assert_eq!(collapse_ws(""), "");
    }
}
