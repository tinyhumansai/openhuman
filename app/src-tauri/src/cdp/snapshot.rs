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
    /// one parsed tree containing the main document and any iframe documents.
    pub async fn capture(cdp: &mut CdpConn, session: &str) -> Result<Self, String> {
        log::debug!("[cdp::snapshot] capture start session={session}");
        let raw = cdp
            .call(
                "DOMSnapshot.captureSnapshot",
                capture_request(),
                Some(session),
            )
            .await
            .map_err(|error| {
                log::warn!("[cdp::snapshot] capture call failed session={session} error={error}");
                error
            })?;
        log::debug!("[cdp::snapshot] capture call complete session={session}");
        let snap: CaptureSnapshot = serde_json::from_value(raw).map_err(|error| {
            log::warn!("[cdp::snapshot] decode failed session={session} error={error}");
            format!("decode DOMSnapshot: {error}")
        })?;
        let snapshot = Self::from_capture(snap);
        log::debug!(
            "[cdp::snapshot] decode complete session={session} nodes={}",
            snapshot.len()
        );
        Ok(snapshot)
    }

    fn from_capture(snap: CaptureSnapshot) -> Self {
        let strings = snap.strings;
        // Merge every document (main frame + all iframes) into a single
        // flat node array. CDP returns each frame as its own document
        // with its own indices; we offset child node ids by the running
        // total so cross-document attr/tag/children lookups stay
        // consistent.
        //
        // Gmail email bodies render inside an iframe so without this
        // merge our scrapers couldn't see message HTML at all. The cost
        // is a slightly larger flat tree, but the snapshot is
        // throwaway per call.
        let mut merged_parent_index: Vec<i32> = Vec::new();
        let mut merged_node_type: Vec<i32> = Vec::new();
        let mut merged_node_name: Vec<i32> = Vec::new();
        let mut merged_node_value: Vec<i32> = Vec::new();
        let mut merged_attributes: Vec<Vec<i32>> = Vec::new();
        for document in snap.documents {
            let doc_offset = merged_node_type.len() as i32;
            let doc_nodes = document.nodes;
            for &p in &doc_nodes.parent_index {
                merged_parent_index.push(if p < 0 { -1 } else { p + doc_offset });
            }
            merged_node_type.extend(doc_nodes.node_type);
            merged_node_name.extend(doc_nodes.node_name);
            merged_node_value.extend(doc_nodes.node_value);
            merged_attributes.extend(doc_nodes.attributes);
            // Pad short vectors so they all match doc_count length —
            // CDP is sparse when no attributes / values exist.
            while merged_node_name.len() < merged_node_type.len() {
                merged_node_name.push(-1);
            }
            while merged_node_value.len() < merged_node_type.len() {
                merged_node_value.push(-1);
            }
            while merged_attributes.len() < merged_node_type.len() {
                merged_attributes.push(Vec::new());
            }
        }
        let nodes = NodeTreeSnap {
            parent_index: merged_parent_index,
            node_type: merged_node_type,
            node_name: merged_node_name,
            node_value: merged_node_value,
            attributes: merged_attributes,
        };
        let count = nodes.node_type.len();
        let mut children: Vec<Vec<usize>> = vec![Vec::new(); count];
        for (i, &p) in nodes.parent_index.iter().enumerate() {
            if p >= 0 && (p as usize) < count {
                children[p as usize].push(i);
            }
        }
        Self {
            strings,
            nodes,
            children,
        }
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

fn capture_request() -> serde_json::Value {
    json!({
        "computedStyles": [],
        "includePaintOrder": false,
        "includeDOMRects": false,
    })
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
    fn capture_request_disables_dom_rects() {
        let request = capture_request();
        assert_eq!(request["includeDOMRects"], false);
        assert_eq!(request["includePaintOrder"], false);
        assert_eq!(request["computedStyles"], json!([]));
    }

    #[test]
    fn from_capture_offsets_documents_and_builds_child_adjacency_without_layout() {
        let capture: CaptureSnapshot = serde_json::from_value(json!({
            "strings": ["DIV", "first", "SPAN", "second"],
            "documents": [
                {
                    "nodes": {
                        "parentIndex": [-1, 0],
                        "nodeType": [1, 3],
                        "nodeName": [0, -1],
                        "nodeValue": [-1, 1]
                    }
                },
                {
                    "nodes": {
                        "parentIndex": [-1, 0],
                        "nodeType": [1, 3],
                        "nodeName": [2, -1],
                        "nodeValue": [-1, 3]
                    }
                }
            ]
        }))
        .expect("snapshot fixture should decode without layout data");

        let snapshot = Snapshot::from_capture(capture);

        assert_eq!(snapshot.len(), 4);
        assert_eq!(snapshot.children(0), &[1]);
        assert_eq!(snapshot.children(2), &[3]);
        assert!(snapshot.children(1).is_empty());
        assert_eq!(snapshot.tag(0), "DIV");
        assert_eq!(snapshot.tag(2), "SPAN");
        assert_eq!(snapshot.text_content(0), "first");
        assert_eq!(snapshot.text_content(2), "second");
    }

    #[test]
    fn collapse_ws_collapses_and_trims() {
        assert_eq!(collapse_ws("  hello   world  "), "hello world");
        assert_eq!(collapse_ws("\n\tfoo\n\n"), "foo");
        assert_eq!(collapse_ws(""), "");
    }
}
