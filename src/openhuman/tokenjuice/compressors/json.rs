//! JSON-array crusher (SmartCrusher).
//!
//! Clean-room port of Headroom's `SmartCrusher` (Apache-2.0), extended with
//! variance-aware row preservation. An array of objects that repeat the same
//! keys is the single most common bloated tool output (API list responses, DB
//! rows, search manifests). Re-rendering it as a table emits each key **once**
//! instead of per row.
//!
//! Up to [`ROW_DROP_THRESHOLD`] rows every value is preserved (faithful
//! reformat). Above the threshold the table is row-dropped — but rather than a
//! blind head/tail window, rows carrying **error indicators** or **numeric
//! outliers** are always kept, so anomalies survive even when the bulk of a
//! homogeneous array is dropped. The router offloads the full original to CCR,
//! so the dropped rows stay recoverable.

use async_trait::async_trait;
use serde_json::Value;
use std::collections::BTreeSet;
use std::fmt::Write as _;

use super::signals::has_error_indicators;
use super::Compressor;
use crate::openhuman::tokenjuice::types::{
    CompressInput, CompressOptions, CompressOutput, CompressorKind,
};

/// Minimum rows before tabular rendering is worth the header overhead.
pub const MIN_ROWS: usize = 3;
/// Above this many rows the table is additionally row-dropped.
pub const ROW_DROP_THRESHOLD: usize = 40;
/// Rows kept from the head when row-dropping.
pub const HEAD_ROWS: usize = 20;
/// Rows kept from the tail when row-dropping.
pub const TAIL_ROWS: usize = 10;
/// Z-score beyond which a numeric cell is treated as an outlier worth keeping.
pub const OUTLIER_SIGMA: f64 = 2.0;

pub struct JsonCompressor;

#[async_trait]
impl Compressor for JsonCompressor {
    fn kind(&self) -> CompressorKind {
        CompressorKind::SmartCrusher
    }

    async fn compress(
        &self,
        input: &CompressInput<'_>,
        _opts: &CompressOptions,
    ) -> Option<CompressOutput> {
        compress(input.content)
    }
}

/// Compress a JSON array-of-objects into a compact table. Returns `None` when
/// the content isn't a uniform-enough array of objects or wouldn't shrink.
pub fn compress(content: &str) -> Option<CompressOutput> {
    let value: Value = serde_json::from_str(content.trim()).ok()?;
    let array = value.as_array()?;
    if array.len() < MIN_ROWS {
        return None;
    }
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
        // Keep head + tail PLUS any anomalous rows (errors / numeric outliers)
        // so the signal in a large homogeneous array survives row-dropping.
        let keep = rows_to_keep(array, &columns, rows.len());
        let mut prev: Option<usize> = None;
        for &i in &keep {
            if let Some(p) = prev {
                let gap = i - p - 1;
                if gap > 0 {
                    let _ = writeln!(out, "[... {gap} row(s) omitted ...]");
                }
            } else if i > 0 {
                let _ = writeln!(out, "[... {i} row(s) omitted ...]");
            }
            let _ = writeln!(out, "{}", rows[i]);
            prev = Some(i);
        }
        if let Some(p) = prev {
            let tail = rows.len().saturating_sub(p + 1);
            if tail > 0 {
                let _ = writeln!(out, "[... {tail} row(s) omitted ...]");
            }
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
        "[tokenjuice][json] {} rows × {} cols, lossy={} ({} -> {} bytes)",
        rows.len(),
        columns.len(),
        lossy,
        content.len(),
        out.len(),
    );
    if lossy {
        Some(CompressOutput::lossy(out, CompressorKind::SmartCrusher))
    } else {
        // All values preserved, but the array→table reformat changes layout.
        Some(CompressOutput::reformatted(
            out,
            CompressorKind::SmartCrusher,
        ))
    }
}

/// Pick the row indices to keep when row-dropping: the head/tail windows plus
/// any row flagged as anomalous (error text or a numeric outlier in any
/// column). Returns ascending, de-duplicated indices.
fn rows_to_keep(array: &[Value], columns: &[String], n: usize) -> Vec<usize> {
    let mut keep: BTreeSet<usize> = BTreeSet::new();
    for i in 0..HEAD_ROWS.min(n) {
        keep.insert(i);
    }
    for i in n.saturating_sub(TAIL_ROWS)..n {
        keep.insert(i);
    }

    // Error-text rows: any string/scalar cell carrying an error indicator.
    for (i, item) in array.iter().enumerate() {
        if let Some(obj) = item.as_object() {
            let row_has_error = obj.values().any(|v| match v {
                Value::String(s) => has_error_indicators(s),
                other => has_error_indicators(&other.to_string()),
            });
            if row_has_error {
                keep.insert(i);
            }
        }
    }

    // Numeric outliers: for each column, compute mean/std over numeric cells and
    // keep rows whose value is beyond OUTLIER_SIGMA. Bounded by a cap so a wide
    // anomalous tail can't defeat the point of dropping.
    for col in columns {
        let nums: Vec<(usize, f64)> = array
            .iter()
            .enumerate()
            .filter_map(|(i, item)| {
                item.as_object()
                    .and_then(|o| o.get(col))
                    .and_then(Value::as_f64)
                    .map(|x| (i, x))
            })
            .collect();
        if nums.len() < 4 {
            continue;
        }
        let mean = nums.iter().map(|(_, x)| x).sum::<f64>() / nums.len() as f64;
        let var = nums.iter().map(|(_, x)| (x - mean).powi(2)).sum::<f64>() / nums.len() as f64;
        let std = var.sqrt();
        if std <= f64::EPSILON {
            continue;
        }
        for (i, x) in nums {
            if ((x - mean) / std).abs() >= OUTLIER_SIGMA {
                keep.insert(i);
            }
        }
    }

    keep.into_iter().collect()
}

/// Render a single cell. Scalars print bare-ish; nested values stay as compact
/// JSON so the table remains lossless.
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
        assert_eq!(out.matches("status").count(), 1, "{out}");
        assert!(out.contains("item number 7"));
        assert!(out.len() < input.len(), "expected shrink");
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
        assert!(c.lossy, "row-dropped output must be lossy");
        assert!(c.text.contains("record number 0"), "{}", c.text);
        assert!(c.text.contains("record number 199"), "{}", c.text);
        assert!(c.text.contains("omitted"));
        assert!(c.text.len() < input.len());
    }

    #[test]
    fn keeps_error_row_in_dropped_middle() {
        // A homogeneous array with a single error row buried in the middle: the
        // SmartCrusher must keep that row even though it's in the drop window.
        let mut rows = Vec::new();
        for i in 0..120 {
            let status = if i == 75 { "error: timeout" } else { "ok" };
            rows.push(format!(
                r#"{{"id":{i},"name":"job {i}","status":"{status}","note":"detail {i}"}}"#
            ));
        }
        let input = format!("[{}]", rows.join(","));
        let c = compress(&input).expect("compresses");
        assert!(c.lossy);
        assert!(
            c.text.contains("job 75"),
            "error row must survive:\n{}",
            c.text
        );
        assert!(c.text.contains("error: timeout"));
    }

    #[test]
    fn keeps_numeric_outlier_row() {
        let mut rows = Vec::new();
        for i in 0..120 {
            // Most latencies ~10ms; row 88 is a 9999ms outlier.
            let latency = if i == 88 { 9999 } else { 10 + (i % 3) };
            rows.push(format!(
                r#"{{"id":{i},"endpoint":"/api/{i}","latency_ms":{latency},"region":"us"}}"#
            ));
        }
        let input = format!("[{}]", rows.join(","));
        let c = compress(&input).expect("compresses");
        assert!(
            c.text.contains("9999"),
            "outlier row must survive:\n{}",
            c.text
        );
    }

    #[test]
    fn non_array_returns_none() {
        assert!(compress(r#"{"a":1}"#).is_none());
        assert!(compress("[1,2,3]").is_none());
        assert!(compress(r#"[{"a":1}]"#).is_none());
    }
}
