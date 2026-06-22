//! JSON-array crusher.
//!
//! Clean-room port of the *lossless* core of headroom's `SmartCrusher`
//! (Apache-2.0): an array of objects that repeat the same keys is the single
//! most common bloated tool output (API list responses, DB rows, search
//! manifests). Re-rendering it as a table emits each key **once** instead of
//! per row, dropping the repeated key names and JSON punctuation.
//!
//! Up to [`ROW_DROP_THRESHOLD`] rows every value is preserved (nested values
//! render as compact JSON in their cell); the array→table reformat changes only
//! layout, so it returns [`Compacted::reformatted`]. Above the threshold the
//! table is additionally **row-dropped** (head + tail kept) and returns
//! [`Compacted::lossy`]. Either way the caller (`compact_tool_output`) offloads
//! the full original to CCR behind a `retrieve_tool_output("<hash>")` footer, so
//! the agent can always fetch the exact original JSON back on demand.

use super::Compacted;
use serde_json::Value;
use std::fmt::Write as _;

/// Minimum rows before tabular rendering is worth the header overhead.
pub const MIN_ROWS: usize = 3;
/// Above this many rows the table is *also* row-dropped: head + tail rows are
/// kept and the full original is offloaded to CCR behind a retrieval marker.
pub const ROW_DROP_THRESHOLD: usize = 40;
/// Rows kept from the head when row-dropping.
pub const HEAD_ROWS: usize = 20;
/// Rows kept from the tail when row-dropping.
pub const TAIL_ROWS: usize = 10;

/// Compress a JSON array-of-objects into a compact table. Returns `None` when
/// the content isn't a uniform-enough array of objects or wouldn't shrink.
///
/// Lossless (only reformats) up to [`ROW_DROP_THRESHOLD`] rows; above it the
/// middle rows are dropped and the result is marked `lossy` so the caller
/// offloads the original to CCR behind the retrieval footer.
pub fn compress(content: &str) -> Option<Compacted> {
    let value: Value = serde_json::from_str(content.trim()).ok()?;
    let array = value.as_array()?;
    if array.len() < MIN_ROWS {
        return None;
    }
    // Every element must be an object for a clean table; mixed arrays bail.
    if !array.iter().all(Value::is_object) {
        return None;
    }

    // Column order = first-seen key order across all rows (union, stable).
    let mut columns: Vec<String> = Vec::new();
    for item in array {
        if let Some(obj) = item.as_object() {
            for key in obj.keys() {
                if !columns.iter().any(|c| c == key) {
                    columns.push(key.clone());
                }
            }
        }
    }
    if columns.len() < 2 {
        return None;
    }

    // Render every row's cells up front so we can choose full vs. row-dropped.
    let mut rows: Vec<String> = Vec::with_capacity(array.len());
    for item in array {
        let obj = item.as_object()?;
        let cells: Vec<String> = columns
            .iter()
            .map(|col| match obj.get(col) {
                // Distinguish a truly absent key (blank) from an explicit null
                // (rendered as `null` by render_cell) so the view stays faithful.
                None => String::new(),
                Some(v) => render_cell(v),
            })
            .collect();
        rows.push(cells.join(" | "));
    }

    let lossy = rows.len() > ROW_DROP_THRESHOLD;
    let mut out = String::with_capacity(content.len());
    let _ = writeln!(
        out,
        "[json table: {} rows × {} cols · blank=absent key · exact original via retrieve footer]",
        rows.len(),
        columns.len()
    );
    let _ = writeln!(out, "{}", columns.join(" | "));

    if lossy {
        // Keep head + tail; the caller offloads the full original to CCR and
        // appends the retrieve_tool_output footer, so the dropped middle stays
        // recoverable. The inline marker here just shows where rows were cut.
        let dropped = rows.len() - HEAD_ROWS - TAIL_ROWS;
        for row in rows.iter().take(HEAD_ROWS) {
            let _ = writeln!(out, "{row}");
        }
        let _ = writeln!(out, "[... {dropped} middle rows omitted ...]");
        for row in rows.iter().skip(rows.len() - TAIL_ROWS) {
            let _ = writeln!(out, "{row}");
        }
    } else {
        for row in &rows {
            let _ = writeln!(out, "{row}");
        }
    }

    let out = out.trim_end().to_string();
    if out.len() >= content.len() {
        return None;
    }
    log::debug!(
        "[compaction][json] {} rows × {} cols, lossy={} ({} -> {} bytes)",
        rows.len(),
        columns.len(),
        lossy,
        content.len(),
        out.len(),
    );
    if lossy {
        Some(Compacted::lossy(out))
    } else {
        // All values preserved, but the array→table reformat changes layout
        // (key order, quoting). Reported as `reformatted` so the caller still
        // offloads the original — the agent can fetch exact JSON bytes back.
        Some(Compacted::reformatted(out))
    }
}

/// Render a single cell. Scalars print bare-ish (strings unquoted unless they
/// contain the column separator); nested values stay as compact JSON so the
/// table remains lossless.
fn render_cell(v: &Value) -> String {
    match v {
        Value::String(s) if !s.contains('|') && !s.contains('\n') => s.clone(),
        Value::Bool(b) => b.to_string(),
        Value::Number(n) => n.to_string(),
        other => serde_json::to_string(other).unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crushes_uniform_array() {
        let mut rows = Vec::new();
        for i in 0..20 {
            rows.push(format!(
                r#"{{"id":{i},"name":"item number {i}","status":"active","owner":"team-alpha"}}"#
            ));
        }
        let input = format!("[{}]", rows.join(","));
        let out = compress(&input).expect("compresses").text;
        // Header + key names appear once, not 20× (column order is whatever
        // serde_json yields — don't assume insertion order here).
        assert_eq!(out.matches("status").count(), 1, "{out}");
        for col in ["id", "name", "status", "owner"] {
            assert!(out.lines().nth(1).unwrap().contains(col), "missing {col}");
        }
        // Data preserved.
        assert!(out.contains("item number 7"));
        assert!(out.contains("19"));
        assert!(out.len() < input.len(), "expected shrink");
    }

    #[test]
    fn preserves_nested_values_losslessly() {
        // Enough rows that the table beats the input even with the header.
        let mut rows = Vec::new();
        for i in 0..8 {
            rows.push(format!(
                r#"{{"id":{i},"tags":["alpha","beta"],"meta":{{"k":{i},"label":"row{i}"}}}}"#
            ));
        }
        let input = format!("[{}]", rows.join(","));
        let out = compress(&input).expect("compresses").text;
        assert!(out.contains(r#"["alpha","beta"]"#), "{out}");
        assert!(out.contains(r#""label":"row3""#), "{out}");
    }

    #[test]
    fn handles_missing_keys() {
        // Enough rows with longish values that dropping repeated keys shrinks.
        let mut rows = Vec::new();
        for i in 0..12 {
            rows.push(format!(
                r#"{{"alpha":{i},"bravo":"value string {i}","charlie":"another value {i}"}}"#
            ));
        }
        rows.push(r#"{"alpha":99}"#.to_string()); // missing bravo/charlie
        let input = format!("[{}]", rows.join(","));
        let out = compress(&input).expect("compresses").text;
        let header = out.lines().nth(1).unwrap();
        for col in ["alpha", "bravo", "charlie"] {
            assert!(header.contains(col), "header missing {col}: {header}");
        }
        assert!(out.len() < input.len());
    }

    #[test]
    fn large_array_row_drops_and_is_marked_lossy() {
        let mut rows = Vec::new();
        for i in 0..200 {
            rows.push(format!(
                r#"{{"id":{i},"name":"record number {i}","status":"active","note":"some detail {i}"}}"#
            ));
        }
        let input = format!("[{}]", rows.join(","));
        let c = compress(&input).expect("compresses");

        // Marked lossy → the caller (compact_tool_output) offloads to CCR and
        // appends the retrieve footer. The CCR round-trip is covered at the
        // mod.rs level (lossy_outputs_are_recoverable).
        assert!(c.lossy, "row-dropped output must be lossy");
        assert!(c.text.contains("middle rows omitted"), "{}", c.text);

        // Head + tail rows survive; the middle is dropped.
        assert!(c.text.contains("record number 0"), "{}", c.text);
        assert!(c.text.contains("record number 199"), "{}", c.text);
        assert!(
            !c.text.contains("record number 100"),
            "middle should be dropped"
        );
        assert!(c.text.len() < input.len());
    }

    #[test]
    fn distinguishes_explicit_null_from_absent_key() {
        // explicit null → "null"; absent key → blank. (Faithfulness: the two
        // must not be conflated.)
        let mut rows = Vec::new();
        for i in 0..10 {
            rows.push(format!(
                r#"{{"id":{i},"note":null,"tag":"long enough value to ensure shrink {i}"}}"#
            ));
        }
        let input = format!("[{}]", rows.join(","));
        let c = compress(&input).expect("compresses");
        // A row with explicit null renders the literal "null" (not blank).
        assert!(c.text.contains("null"), "{}", c.text);
    }

    #[test]
    fn small_table_is_lossless() {
        let mut rows = Vec::new();
        for i in 0..10 {
            rows.push(format!(
                r#"{{"id":{i},"name":"row {i} with a reasonably long value","kind":"sample"}}"#
            ));
        }
        let input = format!("[{}]", rows.join(","));
        let c = compress(&input).expect("compresses");
        assert!(!c.lossy, "a full table drops no data");
        assert!(!c.text.contains("omitted"));
    }

    #[test]
    fn non_array_returns_none() {
        assert!(compress(r#"{"a":1}"#).is_none());
        assert!(compress("[1,2,3]").is_none());
        assert!(compress(r#"[{"a":1}]"#).is_none()); // too few rows
    }
}
